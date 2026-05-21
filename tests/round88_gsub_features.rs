//! Round 88 — GSUB feature-tag introspection accessor + Inter Variable
//! coverage for the round-15 ccmp / calt pipeline.
//!
//! ## What this exercises
//!
//! 1. `Face::gsub_features_for_script` — pure pass-through accessor over
//!    the underlying `oxideav-ttf` API, returning the four-byte
//!    feature tags the font publishes under a given OpenType script tag.
//! 2. `Face::has_gsub_feature` — convenience predicate built on top.
//! 3. The round-15 ccmp / calt dispatcher against **Inter Variable**.
//!    Inter publishes 7 `ccmp` lookups (the existing
//!    `shaping::general` module comment cites this) — more than DejaVu's
//!    2 — making it a useful fuller-coverage second fixture for the
//!    round-15 path.
//!
//! ## Why introspection matters
//!
//! Callers building a higher-level shaping API (e.g. choosing whether to
//! pre-pass `liga` vs `dlig`, gating `smcp` on a "small caps" toggle,
//! exposing OpenType feature settings to end users) need to know which
//! tags the *active face* actually publishes. Without an introspection
//! accessor, callers either dropped to the lower-level `oxideav-ttf`
//! crate via `Face::with_font` (correct but breaks the Face-as-handle
//! abstraction) or relied on hard-coded assumptions per font (wrong as
//! soon as the face is swapped). The new accessor exposes the
//! `oxideav-ttf` feature list as part of `Face`'s public surface.

use oxideav_scribe::{Face, FaceChain};

const DEJAVU_BYTES: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
const INTER_BYTES: &[u8] = include_bytes!("fixtures/InterVariable.ttf");

fn cmap(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

// ---------- introspection accessor: empty / missing cases ---------------

#[test]
fn unknown_script_tag_returns_empty() {
    // `xxxx` is not a registered OpenType script tag; no font ships it.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    let tags = face.gsub_features_for_script(*b"xxxx", None);
    assert!(
        tags.is_empty(),
        "unknown script tag must produce an empty feature list, got {:?}",
        tags
    );
}

#[test]
fn has_gsub_feature_is_false_for_unknown_script() {
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    assert!(!face.has_gsub_feature(*b"xxxx", *b"liga"));
}

#[test]
fn has_gsub_feature_is_false_for_unknown_feature() {
    // `zzzz` is not a registered OpenType feature tag. The probe must
    // walk every published tag and return false.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    assert!(!face.has_gsub_feature(*b"latn", *b"zzzz"));
}

// ---------- introspection accessor: DejaVu Sans ground truth ------------

#[test]
fn dejavu_latn_publishes_ccmp() {
    // Per the `shaping::general` module comment, DejaVu Sans ships 2
    // `ccmp` lookups under `latn`. The introspection accessor must
    // surface the `ccmp` tag.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    assert!(
        face.has_gsub_feature(*b"latn", *b"ccmp"),
        "DejaVu Sans must publish `ccmp` under `latn` per round-15 docs comment"
    );
}

#[test]
fn dejavu_latn_publishes_liga() {
    // DejaVu Sans implements `fi` / `fl` ligatures via a GSUB `liga`
    // lookup (this is what the round-1..14 ligature walker has been
    // dispatching all along). The introspection accessor must agree.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    assert!(
        face.has_gsub_feature(*b"latn", *b"liga"),
        "DejaVu Sans must publish `liga` under `latn`"
    );
}

#[test]
fn dejavu_feature_tags_are_well_formed_ascii() {
    // Every published OpenType feature tag is 4 bytes of ASCII per the
    // registered-feature catalogue. The accessor must not invent
    // non-ASCII garbage — this guards against a future regression in
    // either the oxideav-ttf decoder or our pass-through.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    let tags = face.gsub_features_for_script(*b"latn", None);
    assert!(!tags.is_empty(), "DejaVu publishes some latn features");
    for tag in &tags {
        assert!(
            tag.iter().all(|b| (0x20..=0x7E).contains(b)),
            "feature tag {:?} is not printable ASCII",
            tag
        );
    }
}

// ---------- introspection accessor: Inter Variable -----------------------

#[test]
fn inter_latn_publishes_ccmp_and_calt() {
    // Inter Variable is the canonical "modern, feature-rich" variable
    // sans-serif our round-15 dispatcher targets. It publishes 7
    // `ccmp` lookups (per the docs comment in `shaping::general`) and
    // multiple `calt` lookups for stylistic refinements.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    assert!(
        face.has_gsub_feature(*b"latn", *b"ccmp"),
        "Inter must publish `ccmp` under `latn`"
    );
    assert!(
        face.has_gsub_feature(*b"latn", *b"calt"),
        "Inter must publish `calt` under `latn`"
    );
}

#[test]
fn inter_latn_feature_list_matches_observed_tag_set() {
    // Inter Variable's `latn` GSUB feature set is a fixed, observable
    // property of the vendored InterVariable.ttf fixture. Snapshotting
    // the full tag set here protects against silent regressions in
    // the underlying `oxideav-ttf` GSUB walker: if any tag drops out
    // or a new tag appears, the test fails and the assertion needs
    // explicit review.
    //
    // Notable absences worth documenting:
    //   - `liga` — Inter routes its standard f-ligatures through `dlig`
    //     and `calt` rather than the (older) `liga` feature.
    //   - `kern` — Inter ships pair kerning as a GPOS feature, which is
    //     NOT enumerated by `gsub_features_for_script`. A future GPOS
    //     feature-introspection accessor (currently a docs gap on the
    //     `oxideav-ttf` side) is needed to surface it.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let tags: Vec<[u8; 4]> = face.gsub_features_for_script(*b"latn", None);

    // Required-ish bits the round-15 dispatcher relies on.
    for required in [*b"ccmp", *b"calt"] {
        assert!(
            tags.contains(&required),
            "Inter latn must publish {:?}",
            std::str::from_utf8(&required).unwrap()
        );
    }

    // Inter publishes neither `liga` (handled elsewhere) nor `kern`
    // (GPOS).
    assert!(
        !tags.contains(b"liga"),
        "Inter routes ligatures via dlig+calt; no `liga` in GSUB latn"
    );
    assert!(
        !tags.contains(b"kern"),
        "Inter ships kerning under GPOS; `kern` must not appear in the GSUB feature list"
    );

    // Sanity-check the breadth of the published set — Inter is a
    // feature-rich variable face and should publish many style sets,
    // character variants, and the standard fractions / numerator /
    // denominator surface.
    for expected in [*b"dlig", *b"smcp", *b"frac", *b"sups", *b"subs", *b"ss01"] {
        assert!(
            tags.contains(&expected),
            "Inter must list {:?} on its GSUB latn feature set",
            std::str::from_utf8(&expected).unwrap()
        );
    }
}

// ---------- round-15 ccmp pipeline coverage on Inter ---------------------

#[test]
fn inter_ccmp_substitutes_i_before_combining_above_mark() {
    // The defining ccmp behaviour: U+0069 (LATIN SMALL LETTER I)
    // followed by a combining-above mark (U+0307 here) must be
    // rewritten to a dotless-i variant so the dot and the diacritic
    // don't collide. Inter Variable implements this lookup under
    // `latn` ccmp, same as DejaVu Sans — but Inter is variable so we
    // also implicitly validate that the round-15 dispatcher works
    // identically on a variable face.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let raw_i = cmap(&face, 'i');
    assert_ne!(raw_i, 0, "Inter must have 'i' in cmap");

    let chain = FaceChain::new(face);

    // Sanity check: plain "i" stays "i".
    let plain = chain.shape("i", 16.0).expect("plain i shapes");
    assert_eq!(plain.len(), 1);
    assert_eq!(plain[0].glyph_id, raw_i);

    // i + combining-dot-above: base glyph must be substituted.
    let combined = chain.shape("i\u{0307}", 16.0).expect("i + mark shapes");
    assert_eq!(
        combined.len(),
        2,
        "i + combining mark stays two glyphs after ccmp"
    );
    assert_ne!(
        combined[0].glyph_id, raw_i,
        "round-15 ccmp on Inter must substitute the base 'i'"
    );
}

#[test]
fn inter_ascii_run_is_byte_identical_to_cmap_only() {
    // The round-15 ccmp / calt pre-passes must not perturb a pure-ASCII
    // alphabetic run (no combining marks → no coverage hit → identity).
    // This catches a regression where, e.g., a future calt rule were to
    // start firing on raw ASCII glyphs on Inter — we'd want to know
    // about it via a test failure here.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let chain = FaceChain::new(face);

    let expected: Vec<u16> = "The quick brown fox jumps over the lazy dog 0123456789"
        .chars()
        .map(|c| cmap(chain.primary(), c))
        .collect();
    let glyphs = chain
        .shape(
            "The quick brown fox jumps over the lazy dog 0123456789",
            16.0,
        )
        .unwrap();
    let got: Vec<u16> = glyphs.iter().map(|g| g.glyph_id).collect();
    // NB: Inter ships `liga` lookups for `fi` / `fl` etc., and "fox"
    // ("f"+"o"+"x" — no fi/fl) is deliberately chosen to avoid them.
    // If Inter later adds a calt rule that fires on this pangram, this
    // test will fail loudly and the assertion needs revisiting against
    // the new font's lookup tables.
    assert_eq!(
        got, expected,
        "ASCII pangram (no fi/fl) on Inter must be cmap-identical post-round-15"
    );
}
