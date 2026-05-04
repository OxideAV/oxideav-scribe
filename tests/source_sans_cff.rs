//! End-to-end test that the OTF (CFF) face loader handles a real
//! cubic-Bezier fixture (Source Sans 3 Regular, SIL OFL, ~335 KB,
//! shipped with the `oxideav-otf` crate).
//!
//! We only verify structural sanity (face parses, glyphs decode,
//! `Face::glyph_path` emits a non-empty cubic-bearing path) — pixel
//! work belongs to the downstream rasterizer crate.

use oxideav_core::PathCommand;
use oxideav_scribe::{Face, FaceKind};

const FIXTURE: &[u8] = include_bytes!("fixtures/SourceSans3-Regular.otf");

#[test]
fn from_otf_bytes_round_trip() {
    let face = Face::from_otf_bytes(FIXTURE.to_vec()).expect("face from OTF");
    assert_eq!(face.kind(), FaceKind::Otf);
    assert!(face
        .family_name()
        .map(|f| f.contains("Source Sans"))
        .unwrap_or(false));
    assert_eq!(face.units_per_em(), 1000);
}

#[test]
fn glyph_path_o_emits_cubics() {
    // 'O' in Source Sans is a curved oval — the CFF charstring decodes
    // to a sequence of MoveTo + CubicCurveTo + Close commands. (TT
    // outlines emit quadratics here; CFF emits cubics 1:1 from the
    // Type 2 charstring decode — see Face::glyph_path.)
    let face = Face::from_otf_bytes(FIXTURE.to_vec()).expect("face");
    let gid = face
        .with_otf_font(|font| font.glyph_index('O').expect("'O' must map"))
        .expect("with_otf_font");
    let path = face.glyph_path(gid).expect("'O' has an outline");

    let move_count = path
        .commands
        .iter()
        .filter(|c| matches!(c, PathCommand::MoveTo(_)))
        .count();
    let close_count = path
        .commands
        .iter()
        .filter(|c| matches!(c, PathCommand::Close))
        .count();
    let cubic_count = path
        .commands
        .iter()
        .filter(|c| matches!(c, PathCommand::CubicCurveTo { .. }))
        .count();
    let quad_count = path
        .commands
        .iter()
        .filter(|c| matches!(c, PathCommand::QuadCurveTo { .. }))
        .count();
    assert!(move_count >= 1, "expected ≥1 MoveTo");
    assert_eq!(move_count, close_count, "MoveTo / Close must balance");
    assert!(
        cubic_count >= 1,
        "CFF outline expected to emit ≥1 CubicCurveTo, got {cubic_count}"
    );
    assert_eq!(
        quad_count, 0,
        "CFF outline must not emit QuadCurveTo, got {quad_count}"
    );
}

#[test]
fn ttf_face_rejects_with_otf_font() {
    // Sanity: routing a Face built from OTF bytes through with_font
    // (the TTF path) must fail with WrongFaceKind.
    let face = Face::from_otf_bytes(FIXTURE.to_vec()).expect("face");
    let result = face.with_font(|_font| ());
    assert!(matches!(
        result,
        Err(oxideav_scribe::Error::WrongFaceKind { .. })
    ));
}
