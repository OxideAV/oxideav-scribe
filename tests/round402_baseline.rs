//! Round 402 — `BASE`-table baseline metric accessors
//! (`Face::has_base_table` / `Face::baseline_coord`).
//!
//! The `BASE` table (ISO/IEC 14496-22:2019 §6.3.1) registers, per
//! script, the design-unit offset of each named baseline (roman `romn`,
//! ideographic `ideo`, hanging `hang`, …) so a layout engine can align a
//! Latin run and a CJK run on a common line. Scribe exposes it uniformly
//! over both container kinds: the TTF path resolves at the current
//! variation instance, the OTF/CFF path reads the static coordinate.
//!
//! Fixtures:
//! - **Source Sans 3** (OTF/CFF) ships a `HorizAxis` with `romn`/`ideo`
//!   baselines for DFLT/latn/cyrl/grek and no `VertAxis`.
//! - **DejaVu Sans** (TTF) ships no `BASE` table — the graceful-`None`
//!   path.

use oxideav_scribe::{BaselineAxis, Face};

const DEJAVU_BYTES: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
const SOURCE_SANS_BYTES: &[u8] = include_bytes!("fixtures/SourceSans3-Regular.otf");

fn source_sans() -> Face {
    Face::from_otf_bytes(SOURCE_SANS_BYTES.to_vec()).expect("Source Sans parses")
}

#[test]
fn source_sans_has_base_table() {
    assert!(source_sans().has_base_table());
}

#[test]
fn dejavu_has_no_base_table() {
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    assert!(!face.has_base_table());
    // Every baseline query on a BASE-less face is None, never a panic.
    assert_eq!(
        face.baseline_coord(BaselineAxis::Horizontal, *b"latn", *b"romn"),
        None
    );
    assert_eq!(
        face.baseline_coord(BaselineAxis::Vertical, *b"latn", *b"ideo"),
        None
    );
}

#[test]
fn roman_baseline_is_origin_ideographic_is_below() {
    let face = source_sans();
    // The roman/alphabetic baseline is the font origin.
    assert_eq!(
        face.baseline_coord(BaselineAxis::Horizontal, *b"latn", *b"romn"),
        Some(0)
    );
    // The ideographic em-box bottom sits below it (negative Y).
    let ideo = face
        .baseline_coord(BaselineAxis::Horizontal, *b"latn", *b"ideo")
        .expect("Source Sans publishes an ideo baseline for latn");
    assert!(
        ideo < 0,
        "ideographic baseline should be below romn, got {ideo}"
    );
}

#[test]
fn all_registered_scripts_share_the_source_sans_baselines() {
    let face = source_sans();
    // Source Sans lists DFLT/latn/cyrl/grek in its HorizAxis BaseScriptList,
    // all with the same romn=0 / ideo baseline pair.
    let romn0 = face.baseline_coord(BaselineAxis::Horizontal, *b"latn", *b"romn");
    let ideo0 = face.baseline_coord(BaselineAxis::Horizontal, *b"latn", *b"ideo");
    for sc in [b"DFLT", b"cyrl", b"grek"] {
        assert_eq!(
            face.baseline_coord(BaselineAxis::Horizontal, *sc, *b"romn"),
            romn0
        );
        assert_eq!(
            face.baseline_coord(BaselineAxis::Horizontal, *sc, *b"ideo"),
            ideo0
        );
    }
}

#[test]
fn absent_vertical_axis_returns_none() {
    // Source Sans is a horizontal-layout font: no VertAxis.
    let face = source_sans();
    assert_eq!(
        face.baseline_coord(BaselineAxis::Vertical, *b"latn", *b"romn"),
        None
    );
}

#[test]
fn unlisted_script_and_baseline_return_none() {
    let face = source_sans();
    // A script the BaseScriptList does not carry.
    assert_eq!(
        face.baseline_coord(BaselineAxis::Horizontal, *b"arab", *b"romn"),
        None
    );
    // A baseline tag not in the axis's BaseTagList.
    assert_eq!(
        face.baseline_coord(BaselineAxis::Horizontal, *b"latn", *b"hang"),
        None
    );
}
