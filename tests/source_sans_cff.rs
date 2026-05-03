//! End-to-end test that the cubic-Bezier flattener handles a real
//! CFF/OTF fixture (Source Sans 3 Regular, SIL OFL, ~335 KB,
//! shipped with the `oxideav-otf` crate).
//!
//! We only verify structural sanity (face parses, glyphs decode,
//! the flattener emits non-empty polylines) — pixel-perfect
//! rasterisation is the rasterizer crate's concern.

use oxideav_scribe::{flatten_cubic, Face, FaceKind};

const FIXTURE: &[u8] = include_bytes!("../../oxideav-otf/tests/fixtures/SourceSans3-Regular.otf");

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
fn cubic_flatten_produces_polyline() {
    let face = Face::from_otf_bytes(FIXTURE.to_vec()).expect("face");
    // Look up 'O' (mostly cubic curves) and flatten at 16 px.
    face.with_otf_font(|font| {
        let gid = font.glyph_index('O').expect("'O' must map");
        let outline = font.glyph_outline(gid).expect("outline");
        let scale = 16.0_f32 / face.units_per_em() as f32;
        let flat = flatten_cubic(&outline, scale).expect("flatten produces something");
        assert!(!flat.contours.is_empty(), "no contours after flatten");
        // 'O' is a single closed letter (or two, with the inner
        // counter); each contour should have many subdivision
        // samples after flattening at 16 px.
        for c in &flat.contours {
            assert!(c.len() > 4, "contour too short: {} pts", c.len());
        }
        // Bounds should be roughly 16 px tall (depending on glyph
        // shape; just verify non-trivial extent).
        assert!(flat.bounds.width() > 1.0);
        assert!(flat.bounds.height() > 1.0);
    })
    .expect("with_otf_font");
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
