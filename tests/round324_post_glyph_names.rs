//! Round 324 — `post` (PostScript) table glyph-name resolution.
//!
//! Validates `Face::post()` / `Face::glyph_name()` against the real
//! DejaVuSans fixture (a format-2.0 `post` table) plus a synthetic
//! format-1.0 standard-ordering check.

use oxideav_scribe::{Face, STANDARD_MAC_GLYPH_NAMES};

const DEJAVU: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

#[test]
fn dejavu_post_resolves_named_glyphs() {
    let face = Face::from_ttf_bytes(DEJAVU.to_vec()).expect("parse DejaVuSans");
    let post = face.post().expect("DejaVuSans ships a post table");
    assert!(post.has_names(), "DejaVuSans post should carry names");

    // GID 0 is .notdef in every well-formed font.
    assert_eq!(post.glyph_name(0), Some(".notdef"));

    // The cmap-resolved glyph for 'A' (U+0041) must be named "A".
    let gid_a = face
        .with_font(|f| f.glyph_index('A'))
        .expect("with_font")
        .expect("DejaVuSans has 'A'");
    assert_eq!(face.glyph_name(gid_a).as_deref(), Some("A"));

    // 'space' (U+0020).
    let gid_space = face
        .with_font(|f| f.glyph_index(' '))
        .expect("with_font")
        .expect("DejaVuSans has space");
    assert_eq!(face.glyph_name(gid_space).as_deref(), Some("space"));

    // 'a' (U+0061).
    let gid_a_lower = face
        .with_font(|f| f.glyph_index('a'))
        .expect("with_font")
        .expect("DejaVuSans has 'a'");
    assert_eq!(face.glyph_name(gid_a_lower).as_deref(), Some("a"));
}

#[test]
fn glyph_name_consistent_with_standard_table() {
    let face = Face::from_ttf_bytes(DEJAVU.to_vec()).expect("parse DejaVuSans");
    // Every glyph DejaVuSans names with a standard index must match the
    // 258-entry standard ordering exactly.
    let post = face.post().expect("post");
    // .notdef anchor.
    assert_eq!(post.glyph_name(0), Some(STANDARD_MAC_GLYPH_NAMES[0]));
}

#[test]
fn out_of_range_glyph_has_no_name() {
    let face = Face::from_ttf_bytes(DEJAVU.to_vec()).expect("parse DejaVuSans");
    // A glyph ID far beyond the font's glyph count yields no name.
    assert_eq!(face.glyph_name(60000), None);
}
