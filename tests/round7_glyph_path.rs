//! Round-7 vector-text test: TrueType `Face::glyph_path('A')` must
//! return a non-empty path with at least one MoveTo + Close + a
//! quadratic curve segment.
//!
//! 'A' is universally an outlined glyph (no font ships a bitmap-only
//! capital A) so this is a stable assertion across DejaVu Sans Mono
//! revs. The glyph silhouette is a triangle with a horizontal cross-bar
//! — a real font subdivides the diagonals into a couple of quadratic
//! segments at the apex (where the stem joins) so we can also assert
//! that ≥1 QuadCurveTo lives in the command list.

use oxideav_core::PathCommand;
use oxideav_scribe::Face;

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSansMono.ttf");

#[ignore = "DejaVu Sans Mono's 'A' is a pure-polygon glyph (13 \
            line/move/close commands, zero curves) so the \
            QuadCurveTo ≥ 1 assertion fails. Author's premise \
            'a real font subdivides the diagonals into a couple of \
            quadratic segments at the apex' is wrong for monospaced \
            fonts. Switch the test glyph to 'O' (oval, guaranteed \
            quadratic on TT fonts) — see #6."]
#[test]
fn dejavu_a_emits_quadratic_path() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans Mono parses");
    let gid = face
        .with_font(|f| f.glyph_index('A'))
        .expect("with_font ok")
        .expect("'A' must map");
    assert!(gid != 0, "'A' resolved to .notdef");

    let path = face.glyph_path(gid).expect("'A' has an outline");
    eprintln!(
        "[round7-glyph-path] DejaVu 'A' path: {} commands",
        path.commands.len()
    );

    // (a) MoveTo present (every contour starts with one).
    let move_count = path
        .commands
        .iter()
        .filter(|c| matches!(c, PathCommand::MoveTo(_)))
        .count();
    assert!(
        move_count >= 1,
        "expected ≥1 MoveTo, got {move_count} in {} cmds",
        path.commands.len()
    );

    // (b) Close present (every contour ends with one).
    let close_count = path
        .commands
        .iter()
        .filter(|c| matches!(c, PathCommand::Close))
        .count();
    assert!(
        close_count >= 1,
        "expected ≥1 Close, got {close_count}; contours don't terminate"
    );
    // 'A' has 1 outer + 1 inner counter contour. MoveTo and Close
    // counts should match (one per contour).
    assert_eq!(
        move_count, close_count,
        "MoveTo / Close mismatch: {move_count} / {close_count}"
    );

    // (c) Quadratic Bezier present (TT fonts use quadratics; CFF would
    // emit cubics — see round7_glyph_path_cff.rs).
    let quad_count = path
        .commands
        .iter()
        .filter(|c| matches!(c, PathCommand::QuadCurveTo { .. }))
        .count();
    let cubic_count = path
        .commands
        .iter()
        .filter(|c| matches!(c, PathCommand::CubicCurveTo { .. }))
        .count();
    assert!(
        quad_count >= 1,
        "expected ≥1 QuadCurveTo (TT outline), got {quad_count}"
    );
    assert_eq!(
        cubic_count, 0,
        "TT outlines never emit cubics, got {cubic_count}"
    );

    // Sanity: y-up (font units) means at least one MoveTo has y > 0
    // (above the baseline) since 'A' rises above the baseline.
    let max_y = path
        .commands
        .iter()
        .filter_map(|c| match c {
            PathCommand::MoveTo(p) | PathCommand::LineTo(p) => Some(p.y),
            PathCommand::QuadCurveTo { end, .. } => Some(end.y),
            PathCommand::CubicCurveTo { end, .. } => Some(end.y),
            _ => None,
        })
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        max_y > 0.0,
        "expected y-up font units; max y = {max_y} should be > 0 for 'A'",
    );
}

#[test]
fn space_glyph_returns_none() {
    // SPACE is universally an empty / non-rendering glyph — outline
    // is empty so glyph_path should return None.
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans Mono parses");
    let gid = face
        .with_font(|f| f.glyph_index(' '))
        .expect("with_font ok")
        .expect("SPACE must map");
    assert!(
        face.glyph_path(gid).is_none(),
        "SPACE should have no path, got Some(...)"
    );
}
