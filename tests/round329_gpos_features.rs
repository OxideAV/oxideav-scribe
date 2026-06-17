//! Round 329 — GPOS feature-tag introspection accessor.
//!
//! ## What this exercises
//!
//! 1. `Face::gpos_features_for_script` — the positioning-table mirror of
//!    `Face::gsub_features_for_script` (round 88). Returns the four-byte
//!    feature tags the active face publishes under a given OpenType
//!    script tag, walking the same ScriptList / FeatureList / LangSys
//!    substructure but over the **GPOS** table.
//! 2. `Face::has_gpos_feature` — the convenience predicate built on top,
//!    mirroring `Face::has_gsub_feature`.
//!
//! ## Why the GPOS mirror matters
//!
//! Pair kerning (`kern`), mark attachment (`mark` / `mkmk`), cursive
//! joining (`curs`) and capital spacing (`cpsp`) all live in GPOS, not
//! GSUB. A higher-level shaping API that wants to gate behaviour on
//! "does this face ship real kerning?" or "does it carry capital
//! spacing?" could not answer with the GSUB accessor alone — the round-88
//! `inter_latn_feature_list_matches_observed_tag_set` test even
//! documents this as a deliberate gap (`kern` is GPOS, so it never
//! appears in the GSUB feature list). This accessor closes that gap.
//!
//! Ground-truth tag sets below are an observable property of the two
//! vendored fixtures; snapshotting them protects against a silent
//! regression in the underlying `oxideav-ttf` GPOS walker.

use oxideav_scribe::Face;

const DEJAVU_BYTES: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
const INTER_BYTES: &[u8] = include_bytes!("fixtures/InterVariable.ttf");

// ---------- introspection accessor: empty / missing cases ---------------

#[test]
fn unknown_script_tag_returns_empty() {
    // `xxxx` is not a registered OpenType script tag; no font ships it.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    let tags = face.gpos_features_for_script(*b"xxxx", None);
    assert!(
        tags.is_empty(),
        "unknown script tag must produce an empty GPOS feature list, got {:?}",
        tags
    );
}

#[test]
fn has_gpos_feature_is_false_for_unknown_script() {
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    assert!(!face.has_gpos_feature(*b"xxxx", *b"kern"));
}

#[test]
fn has_gpos_feature_is_false_for_unknown_feature() {
    // `zzzz` is not a registered OpenType feature tag. The probe must
    // walk every published tag and return false.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    assert!(!face.has_gpos_feature(*b"latn", *b"zzzz"));
}

// ---------- introspection accessor: DejaVu Sans ground truth ------------

#[test]
fn dejavu_latn_publishes_kern_mark_mkmk() {
    // DejaVu Sans ships pair kerning plus mark-to-base / mark-to-mark
    // attachment under `latn` GPOS. These are the lookups the round-1..N
    // GPOS positioning paths have been dispatching all along; the
    // introspection accessor must agree.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    for required in [*b"kern", *b"mark", *b"mkmk"] {
        assert!(
            face.has_gpos_feature(*b"latn", required),
            "DejaVu Sans must publish {:?} under `latn` GPOS",
            std::str::from_utf8(&required).unwrap()
        );
    }
}

#[test]
fn dejavu_kern_is_gpos_only_not_gsub() {
    // `kern` is a positioning feature. It must surface through the GPOS
    // accessor and must NOT appear in the GSUB feature list — the two
    // tables are distinct surfaces and the round-88 GSUB accessor only
    // ever reports substitution features.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    assert!(
        face.has_gpos_feature(*b"latn", *b"kern"),
        "kern lives in GPOS"
    );
    assert!(
        !face.has_gsub_feature(*b"latn", *b"kern"),
        "kern must not appear in the GSUB feature list"
    );
}

#[test]
fn dejavu_dflt_script_publishes_kern() {
    // The catch-all `DFLT` script also carries the kern feature on
    // DejaVu — the script-walk must resolve a DefaultLangSys under DFLT
    // exactly as it does under latn.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    assert!(face.has_gpos_feature(*b"DFLT", *b"kern"));
}

#[test]
fn dejavu_feature_tags_are_well_formed_ascii() {
    // Every published OpenType feature tag is 4 bytes of printable ASCII
    // per the registered-feature catalogue. The accessor must not invent
    // non-ASCII garbage — guards against a future regression in either
    // the oxideav-ttf GPOS decoder or our pass-through.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    let tags = face.gpos_features_for_script(*b"latn", None);
    assert!(!tags.is_empty(), "DejaVu publishes some latn GPOS features");
    for tag in &tags {
        assert!(
            tag.iter().all(|b| (0x20..=0x7E).contains(b)),
            "GPOS feature tag {:?} is not printable ASCII",
            tag
        );
    }
}

// ---------- introspection accessor: Inter Variable -----------------------

#[test]
fn inter_latn_publishes_cpsp_and_kern() {
    // Inter Variable is the canonical "modern, feature-rich" variable
    // sans-serif. Its `latn` GPOS surface adds `cpsp` (capital spacing)
    // on top of the kern / mark / mkmk set DejaVu carries.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    for required in [*b"cpsp", *b"kern", *b"mark", *b"mkmk"] {
        assert!(
            face.has_gpos_feature(*b"latn", required),
            "Inter must publish {:?} under `latn` GPOS",
            std::str::from_utf8(&required).unwrap()
        );
    }
}

#[test]
fn inter_latn_gpos_list_matches_observed_tag_set() {
    // Inter Variable's `latn` GPOS feature set is a fixed, observable
    // property of the vendored InterVariable.ttf fixture. Snapshotting
    // the full tag set protects against silent regressions in the
    // underlying `oxideav-ttf` GPOS walker: if any tag drops out or a
    // new tag appears, the test fails and the assertion needs explicit
    // review.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let tags: Vec<[u8; 4]> = face.gpos_features_for_script(*b"latn", None);

    let mut as_str: Vec<String> = tags
        .iter()
        .map(|t| String::from_utf8_lossy(t).into_owned())
        .collect();
    as_str.sort();
    assert_eq!(
        as_str,
        vec![
            "cpsp".to_string(),
            "kern".to_string(),
            "mark".to_string(),
            "mkmk".to_string(),
        ],
        "Inter latn GPOS feature set changed — review against the fixture"
    );
}

#[test]
fn inter_capital_spacing_is_gpos_only() {
    // `cpsp` is a positioning feature; like `kern` it must surface only
    // through the GPOS accessor, never the GSUB one.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    assert!(
        face.has_gpos_feature(*b"latn", *b"cpsp"),
        "Inter ships capital spacing under GPOS"
    );
    assert!(
        !face.has_gsub_feature(*b"latn", *b"cpsp"),
        "cpsp must not appear in the GSUB feature list"
    );
}

// ---------- accessor / predicate consistency -----------------------------

#[test]
fn predicate_agrees_with_list_for_every_published_tag() {
    // `has_gpos_feature(s, t)` must be true for exactly the tags the list
    // accessor publishes (under the default LangSys). This guards the
    // convenience predicate against drifting from the underlying list.
    for bytes in [DEJAVU_BYTES, INTER_BYTES] {
        let face = Face::from_ttf_bytes(bytes.to_vec()).expect("font parses");
        let tags = face.gpos_features_for_script(*b"latn", None);
        for tag in &tags {
            assert!(
                face.has_gpos_feature(*b"latn", *tag),
                "predicate disagrees with list for {:?}",
                std::str::from_utf8(tag).unwrap()
            );
        }
        // And a tag the list does not contain must be reported absent.
        assert!(!tags.contains(b"zzzz"));
        assert!(!face.has_gpos_feature(*b"latn", *b"zzzz"));
    }
}
