//! Round-7 vector-text test: `Shaper::shape_to_paths` returns one
//! `(face_idx, Node, Transform2D)` per rendered glyph, with the
//! second glyph translated to the right of the first (positive
//! advance), and outline glyphs come back as `Node::Path`.

use oxideav_core::Node;
use oxideav_scribe::{Face, FaceChain, Shaper};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSansMono.ttf");

#[test]
fn shape_hi_returns_two_path_nodes_with_increasing_x() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans Mono parses");
    let chain = FaceChain::new(face);

    let placed = Shaper::shape_to_paths(&chain, "Hi", 32.0);
    eprintln!("[round7-shape-to-paths] {} placed glyphs", placed.len());

    // (a) 2 glyphs (H + i), both rendering, no ligatures in DejaVu
    // Sans Mono for "Hi".
    assert_eq!(
        placed.len(),
        2,
        "expected 2 placed glyphs for 'Hi', got {}",
        placed.len()
    );

    // (b) Both nodes are PathNodes (outline glyphs, no bitmaps in
    // DejaVu Sans Mono).
    for (i, (face_idx, node, _)) in placed.iter().enumerate() {
        assert_eq!(*face_idx, 0, "single-face chain → face_idx 0");
        assert!(
            matches!(node, Node::Path(_)),
            "glyph #{i} is not a PathNode — got {node:?}",
        );
    }

    // (c) Second node's translation X is positive (advances rightward).
    let t1 = placed[1].2;
    eprintln!(
        "[round7-shape-to-paths] glyph 1 translate = ({}, {})",
        t1.e, t1.f
    );
    assert!(
        t1.e > 0.0,
        "second glyph should advance rightward (translate.e = {}, expected > 0)",
        t1.e
    );
    // First glyph sits at pen origin (X=0).
    let t0 = placed[0].2;
    assert_eq!(
        t0.e, 0.0,
        "first glyph should sit at pen origin (translate.e = {})",
        t0.e
    );

    // Sanity: the per-glyph fill is the default black solid; replace
    // it via the consumer's downstream pipeline if a different colour
    // is needed.
    if let Node::Path(p) = &placed[0].1 {
        assert!(p.fill.is_some(), "glyph_node should ship a default fill");
    }
}

#[test]
fn empty_string_returns_empty_vec() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans Mono parses");
    let chain = FaceChain::new(face);
    let placed = Shaper::shape_to_paths(&chain, "", 32.0);
    assert!(placed.is_empty(), "empty string must produce 0 glyphs");
}

#[test]
fn space_does_not_emit_a_node() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("DejaVu Sans Mono parses");
    let chain = FaceChain::new(face);
    // "A B" — the space has no rendering, so it should be skipped and
    // we should get exactly 2 placed nodes ('A' + 'B'), not 3.
    let placed = Shaper::shape_to_paths(&chain, "A B", 32.0);
    eprintln!(
        "[round7-shape-to-paths-space] {} placed glyphs",
        placed.len()
    );
    assert_eq!(
        placed.len(),
        2,
        "expected 2 placed glyphs (skipping space), got {}",
        placed.len()
    );
    // The 'B' must be advanced past the space — translate.e of glyph 1
    // should be ≥ ~2 advance widths (A + space) at 32 px.
    let bx = placed[1].2.e;
    assert!(
        bx > 16.0,
        "'B' should sit far past the space (got X={}, expected > 16)",
        bx
    );
}
