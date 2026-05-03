//! Round-7 vector-text test: CFF/OTF `Face::glyph_path('A')` must
//! emit cubic Beziers (the Type 2 charstring decode is cubic-native;
//! the converter is a 1:1 mapping with no on/off-curve dance).

use oxideav_core::PathCommand;
use oxideav_scribe::Face;

const FIXTURE: &[u8] = include_bytes!("fixtures/SourceSans3-Regular.otf");

#[test]
fn source_sans_a_emits_cubic_path() {
    let face = Face::from_otf_bytes(FIXTURE.to_vec()).expect("Source Sans 3 parses");
    let gid = face
        .with_otf_font(|f| f.glyph_index('A'))
        .expect("with_otf_font ok")
        .expect("'A' must map");
    assert!(gid != 0, "'A' resolved to .notdef");

    let path = face.glyph_path(gid).expect("'A' has an outline");
    eprintln!(
        "[round7-glyph-path-cff] Source Sans 'A' path: {} commands",
        path.commands.len()
    );

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

    assert!(move_count >= 1, "expected ≥1 MoveTo, got {move_count}");
    assert!(close_count >= 1, "expected ≥1 Close, got {close_count}");
    assert_eq!(
        move_count, close_count,
        "MoveTo / Close mismatch: {move_count} / {close_count}"
    );
    assert!(
        cubic_count >= 1,
        "expected ≥1 CubicCurveTo (CFF outline), got {cubic_count}"
    );
    assert_eq!(
        quad_count, 0,
        "CFF outlines never emit quadratics, got {quad_count}"
    );
}
