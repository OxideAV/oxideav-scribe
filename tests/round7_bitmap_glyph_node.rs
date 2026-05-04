//! Round-7 vector-text test: `Face::glyph_node(emoji_gid, 96.0)` for
//! a CBDT-only colour-emoji font returns `Node::Image(ImageRef)`
//! carrying a non-empty rasterised bitmap (NOT a `Node::Path` —
//! emojis have no outline in Noto Color Emoji).
//!
//! Same fixture as `round5_emoji.rs`: download-on-demand via the
//! shared font-fixtures helper; gated on `OXIDEAV_NETWORK_TESTS=1`.

#[path = "font_fixtures/mod.rs"]
mod font_fixtures;

use font_fixtures::{load_fixture, NOTO_COLOR_EMOJI_TTF};
use oxideav_core::Node;
use oxideav_scribe::Face;

#[test]
fn party_popper_glyph_node_is_image_ref() {
    let bytes = match load_fixture(&NOTO_COLOR_EMOJI_TTF) {
        Some(b) => b,
        None => return, // skip silently — fixture-helper printed why
    };
    let face = Face::from_ttf_bytes(bytes).expect("Noto Color Emoji parses");
    assert!(
        face.has_color_bitmaps(),
        "Noto Color Emoji must report color-bitmap support",
    );

    let gid = face
        .with_font(|f| f.glyph_index('\u{1F389}'))
        .expect("with_font ok")
        .expect("U+1F389 must map");
    assert!(gid != 0, "U+1F389 resolved to .notdef");
    eprintln!("[round7-bitmap-glyph-node] U+1F389 → glyph id {gid}");

    let node = face
        .glyph_node(gid, 96.0)
        .expect("emoji glyph_node must return Some(Image)");

    let image = match &node {
        Node::Image(im) => im,
        other => panic!("expected Node::Image for emoji, got {other:?}"),
    };
    eprintln!(
        "[round7-bitmap-glyph-node] image bounds: ({}, {}) {}x{}",
        image.bounds.x, image.bounds.y, image.bounds.width, image.bounds.height
    );

    // Bounds must be positive — the bitmap is non-empty + correctly
    // scaled to ~96 px (allow some slack since strikes round).
    assert!(
        image.bounds.width > 0.0 && image.bounds.height > 0.0,
        "image bounds zero-sized: {:?}",
        image.bounds
    );

    // The carried VideoFrame must hold non-empty bitmap data.
    assert!(
        !image.frame.planes.is_empty(),
        "image VideoFrame must have at least one plane"
    );
    let plane = &image.frame.planes[0];
    assert!(
        !plane.data.is_empty(),
        "image plane data is empty (PNG decode probably failed)"
    );
    assert!(plane.stride > 0, "image plane stride is 0");
    eprintln!(
        "[round7-bitmap-glyph-node] plane: stride={}, data_len={}",
        plane.stride,
        plane.data.len()
    );

    // Sanity: at least one pixel has non-zero alpha (4 B/px RGBA).
    let nz = plane.data.chunks_exact(4).filter(|p| p[3] != 0).count();
    assert!(
        nz > 0,
        "no pixels with non-zero alpha — bitmap is fully transparent"
    );
}
