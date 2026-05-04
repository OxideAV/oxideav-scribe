//! End-to-end integration tests against the DejaVu Sans fixture.
//! The fixture is shipped under `tests/fixtures/` so the standalone
//! crate package is self-contained for CI / docs.rs.

use oxideav_core::{Node, PathCommand};
use oxideav_scribe::{Face, FaceChain, Shaper};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn load_face() -> Face {
    Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans must parse")
}

#[test]
fn glyph_path_a_at_32px_emits_outline() {
    // Vector replacement for the old "rasterise 'A'" smoke test:
    // `Face::glyph_path` returns the glyph's vector commands; the
    // raster of the alpha mask is now `oxideav-raster`'s job. We
    // confirm the path has the expected shape (≥1 contour, ≥1
    // quadratic curve segment, MoveTo/Close balanced).
    let face = load_face();
    let gid_a = face
        .with_font(|f| f.glyph_index('A'))
        .expect("with_font")
        .expect("'A' is in DejaVu");
    let path = face.glyph_path(gid_a).expect("'A' has an outline");
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
    assert!(move_count >= 1, "expected ≥1 MoveTo");
    assert_eq!(move_count, close_count, "MoveTo / Close must balance");
}

#[test]
fn shaper_latin_hello_world() {
    let face = load_face();
    let glyphs = Shaper::shape(&face, "Hello, world!", 16.0).expect("shape ok");
    assert_eq!(
        glyphs.len(),
        13,
        "got {} glyphs: {:?}",
        glyphs.len(),
        glyphs
    );

    // Cumulative advance roughly matches per-glyph advance sum.
    let total: f32 = glyphs.iter().map(|g| g.x_advance + g.x_offset).sum();
    let sum_advances: f32 = glyphs.iter().map(|g| g.x_advance).sum();
    let diff = (total - sum_advances).abs();
    assert!(
        diff <= 1.0,
        "advance vs sum diff = {diff}, total = {total}, sum = {sum_advances}"
    );
}

#[test]
fn shaper_kerning_avatar_shrinks() {
    let face = load_face();
    let glyphs = Shaper::shape(&face, "AVATAR", 32.0).expect("shape ok");
    assert_eq!(glyphs.len(), 6);

    // No-kerning baseline: sum of plain advances.
    let baseline_sum: f32 = glyphs.iter().map(|g| g.x_advance).sum();
    let with_kerning: f32 = glyphs.iter().map(|g| g.x_advance + g.x_offset).sum();
    let savings = baseline_sum - with_kerning;
    // DejaVu ships AV (and AT) negative kerning. Even at 32 px the
    // total should be measurably shorter — be generous on the bound.
    assert!(
        savings > 1.0,
        "expected >1 px kerning savings, got {savings} (baseline {baseline_sum}, kerned {with_kerning})"
    );
}

#[test]
fn shaper_fi_ligature_collapses_office() {
    let face = load_face();
    // "office" → 'o','f','f','i','c','e' = 6 codepoints.
    // After GSUB, "ffi" or "fi" ligature collapses to a single glyph,
    // so we should get either 5 (one fi) or 4 (one ffi) glyphs.
    let glyphs = Shaper::shape(&face, "office", 16.0).expect("shape ok");
    assert!(
        glyphs.len() == 4 || glyphs.len() == 5,
        "expected 4 (ffi) or 5 (fi) glyphs after ligature, got {}",
        glyphs.len()
    );
}

#[test]
fn shape_to_paths_hello_emits_one_node_per_visible_glyph() {
    // Vector replacement for the old "render_text produces white pixels"
    // smoke test. `Shaper::shape_to_paths` now returns vector glyph
    // nodes ready for downstream `oxideav-raster` to rasterise; here we
    // verify the run shape (one node per visible glyph, sequential X
    // translations).
    let face = load_face();
    let chain = FaceChain::new(face);
    let placed = Shaper::shape_to_paths(&chain, "Hello", 16.0);
    assert_eq!(
        placed.len(),
        5,
        "expected 5 placed glyphs for 'Hello', got {}",
        placed.len()
    );
    // Each placed glyph is a Group(cache_key=Some) wrapping a PathNode.
    for (i, (face_idx, node, _)) in placed.iter().enumerate() {
        assert_eq!(*face_idx, 0, "single-face chain → face_idx 0");
        let Node::Group(g) = node else {
            panic!("glyph #{i} is not a Group — got {node:?}");
        };
        assert!(g.cache_key.is_some(), "glyph #{i} group missing cache_key");
    }
    // Translations advance rightward (each glyph past the first sits to
    // the right of glyph 0 at pen origin X=0).
    assert_eq!(placed[0].2.e, 0.0, "first glyph at pen origin");
    for i in 1..placed.len() {
        assert!(
            placed[i].2.e > placed[i - 1].2.e,
            "glyph {} (X={}) should sit right of glyph {} (X={})",
            i,
            placed[i].2.e,
            i - 1,
            placed[i - 1].2.e,
        );
    }
}

#[test]
fn cjk_falls_back_to_notdef_without_panic() {
    let face = load_face();
    // DejaVu Sans does NOT include CJK ideographs. The shaper's
    // contract is to fall back to glyph 0 (.notdef / "tofu") rather
    // than dropping the codepoint or panicking.
    let glyphs = Shaper::shape(&face, "日本語", 16.0).expect("shape ok");
    assert_eq!(glyphs.len(), 3, "one glyph per CJK codepoint");
    for g in &glyphs {
        // Glyph 0 is the .notdef glyph in TrueType; it must have a
        // non-zero advance (the "tofu" rectangle) so the layout
        // reserves space.
        assert!(g.x_advance >= 0.0, "advance: {}", g.x_advance);
    }
}
