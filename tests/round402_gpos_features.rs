//! Round 402 — GPOS feature-tag introspection accessors, the
//! positioning-table mirror of the round-88 GSUB surface.
//!
//! ## What this exercises
//!
//! 1. `Face::gpos_features_for_script` — pass-through accessor over the
//!    `oxideav-ttf` GPOS ScriptList / FeatureList walk, returning the
//!    four-byte feature tags the font publishes under an OpenType script
//!    tag (`kern`, `mark`, `mkmk`, `cpsp`, …).
//! 2. `Face::has_gpos_feature` — convenience predicate over it.
//! 3. `Face::layout_features_for_script` — the de-duplicated union of the
//!    GSUB and GPOS tag sets, so a higher-level "which OpenType features
//!    can I toggle?" surface does not have to care which table realises a
//!    given tag.
//!
//! Fixtures: DejaVu Sans (kern/mark/mkmk under many scripts), Source Sans
//! 3 (kern/mark/mkmk/size — a CFF-flavour OTF confirming the accessor
//! spans both container kinds is *not* claimed here; the TTF path rejects
//! OTF, so the OTF face returns the empty vec, which we assert), and
//! Inter Variable (adds `cpsp` — capital spacing).

use oxideav_scribe::Face;

const DEJAVU_BYTES: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
const INTER_BYTES: &[u8] = include_bytes!("fixtures/InterVariable.ttf");
const SOURCE_SANS_BYTES: &[u8] = include_bytes!("fixtures/SourceSans3-Regular.otf");

fn dejavu() -> Face {
    Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu Sans parses")
}
fn inter() -> Face {
    Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter Variable parses")
}

// ---------- empty / missing cases ---------------------------------------

#[test]
fn unknown_script_returns_empty() {
    let face = dejavu();
    // A four-byte tag no font registers.
    assert!(face.gpos_features_for_script(*b"zzzz", None).is_empty());
    assert!(!face.has_gpos_feature(*b"zzzz", *b"kern"));
}

#[test]
fn otf_cff_face_falls_through_to_empty() {
    // The Face GPOS accessor uses the TTF path (mirroring the GSUB
    // accessor's documented behaviour); a CFF-flavour OTF returns the
    // empty vec even though the underlying font *does* carry GPOS.
    let face = Face::from_otf_bytes(SOURCE_SANS_BYTES.to_vec()).expect("Source Sans parses");
    assert!(face.gpos_features_for_script(*b"latn", None).is_empty());
}

// ---------- DejaVu: kern / mark / mkmk ----------------------------------

#[test]
fn dejavu_latin_publishes_kern_mark_mkmk() {
    let face = dejavu();
    let feats = face.gpos_features_for_script(*b"latn", None);
    for tag in [b"kern", b"mark", b"mkmk"] {
        assert!(
            feats.contains(tag),
            "DejaVu latn GPOS should publish {:?}, got {:?}",
            std::str::from_utf8(tag),
            feats
                .iter()
                .map(|t| std::str::from_utf8(t).unwrap_or("????").to_string())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn has_gpos_feature_predicate_matches_the_list() {
    let face = dejavu();
    assert!(face.has_gpos_feature(*b"latn", *b"kern"));
    assert!(face.has_gpos_feature(*b"cyrl", *b"kern"));
    // DejaVu ships no `cpsp` capital-spacing lookup.
    assert!(!face.has_gpos_feature(*b"latn", *b"cpsp"));
    // GSUB tag under the GPOS query must miss.
    assert!(!face.has_gpos_feature(*b"latn", *b"liga"));
}

// ---------- Inter: cpsp -------------------------------------------------

#[test]
fn inter_latin_publishes_cpsp() {
    let face = inter();
    let feats = face.gpos_features_for_script(*b"latn", None);
    assert!(
        feats.contains(b"cpsp"),
        "Inter latn GPOS should publish cpsp, got {feats:?}"
    );
    assert!(face.has_gpos_feature(*b"latn", *b"kern"));
}

// ---------- combined GSUB ∪ GPOS ----------------------------------------

#[test]
fn layout_features_union_is_sorted_deduped_superset() {
    let face = dejavu();
    let gsub = face.gsub_features_for_script(*b"latn", None);
    let gpos = face.gpos_features_for_script(*b"latn", None);
    let union = face.layout_features_for_script(*b"latn", None);

    // Sorted + de-duplicated.
    assert!(
        union.windows(2).all(|w| w[0] < w[1]),
        "union must be strictly sorted: {union:?}"
    );
    // Superset of both inputs.
    for t in gsub.iter().chain(gpos.iter()) {
        assert!(union.contains(t), "union missing {t:?}");
    }
    // `kern` is GPOS-only; a GSUB tag such as `liga` (if present) also lands.
    assert!(union.contains(b"kern"));
}

#[test]
fn layout_features_empty_when_neither_table_lists_script() {
    let face = dejavu();
    assert!(face.layout_features_for_script(*b"zzzz", None).is_empty());
}
