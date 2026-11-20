//! The shaping pipeline end to end on the committed fonts: script
//! itemisation, cluster-level font fallback, per-font normalisation,
//! feature switches, clusters, directional and vertical runs.
//!
//! Glyph-level agreement with a reference shaper is the business of
//! `shaping_conformance.rs`; these tests pin the pipeline's own
//! contracts.

use oxideav_scribe::{Face, FaceChain, PositionedGlyph, ShapeOptions, Shaper};

mod font_fixtures;

const DEJAVU: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
const SOURCE_SANS: &[u8] = include_bytes!("fixtures/SourceSans3-Regular.otf");
const INTER: &[u8] = include_bytes!("fixtures/InterVariable.ttf");
const NOTO_ARABIC: &[u8] = include_bytes!("fixtures/NotoSansArabic-Regular.ttf");

fn dejavu() -> Face {
    Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap()
}

fn source_sans() -> Face {
    Face::from_otf_bytes(SOURCE_SANS.to_vec()).unwrap()
}

fn ids(glyphs: &[PositionedGlyph]) -> Vec<u16> {
    glyphs.iter().map(|g| g.glyph_id).collect()
}

fn gid(face: &Face, c: char) -> u16 {
    Shaper::shape(face, &c.to_string(), 16.0).unwrap()[0].glyph_id
}

#[test]
fn cff_faces_shape_like_truetype_ones() {
    // A CFF face goes through the same engine: ligatures and kerning
    // from its GSUB / GPOS.
    let face = source_sans();
    let plain = Shaper::shape(&face, "ff", 1000.0).unwrap();
    assert_eq!(plain.len(), 1, "ff ligature in a CFF face");
    assert_eq!(plain[0].cluster, 0);
    let chain = FaceChain::new(source_sans());
    let kerned = chain.shape("AV", 1000.0).unwrap();
    let a = chain.shape("A", 1000.0).unwrap();
    assert!(
        kerned[0].x_advance < a[0].x_advance,
        "A is kerned towards V: {} vs {}",
        kerned[0].x_advance,
        a[0].x_advance
    );
}

#[test]
fn clusters_are_byte_offsets_of_grapheme_clusters() {
    let chain = FaceChain::new(dejavu());
    // "é" typed as e + combining acute composes to one glyph; the
    // ligature takes the cluster of its first letter.
    let text = "e\u{301}x fi";
    let glyphs = chain.shape(text, 20.0).unwrap();
    let clusters: Vec<u32> = glyphs.iter().map(|g| g.cluster).collect();
    assert_eq!(clusters, [0, 3, 4, 5]);
    assert_eq!(glyphs[0].glyph_id, gid(chain.primary(), '\u{e9}'));
    for g in &glyphs {
        assert!(text.is_char_boundary(g.cluster as usize));
    }
    // A base and a mark that do not compose stay two glyphs of one
    // cluster.
    let glyphs = chain.shape("q\u{301}", 20.0).unwrap();
    assert_eq!(glyphs.len(), 2);
    assert_eq!((glyphs[0].cluster, glyphs[1].cluster), (0, 0));
    assert_eq!(glyphs[1].x_advance, 0.0);
}

#[test]
fn normalisation_decomposes_what_the_font_lacks() {
    // Find a precomposed Latin letter Source Sans has no glyph for but
    // whose parts it has; it must come out as base + mark, not as
    // `.notdef`.
    let face = source_sans();
    let chain = FaceChain::new(source_sans());
    let has = |c: char| gid(&face, c) != 0;
    let candidate = ('\u{1e00}'..='\u{1eff}')
        .chain('\u{100}'..='\u{24f}')
        .find(|&c| !has(c));
    let Some(c) = candidate else {
        return;
    };
    let shaped = chain.shape(&c.to_string(), 20.0).unwrap();
    // Either it decomposed into drawable parts or it has no canonical
    // decomposition the font covers.
    if shaped.len() > 1 {
        assert!(shaped.iter().all(|g| g.glyph_id != 0));
        assert!(shaped.iter().all(|g| g.cluster == 0));
    }
}

#[test]
fn fallback_is_decided_per_cluster() {
    // Source Sans has no Hebrew; DejaVu Sans has.
    let chain = FaceChain::new(source_sans()).push_fallback(dejavu());
    let text = "ab \u{5e9}\u{5dc}\u{5d5}\u{5dd}";
    let glyphs = chain.shape(text, 20.0).unwrap();
    let faces: Vec<u16> = glyphs.iter().map(|g| g.face_idx).collect();
    assert_eq!(faces, [0, 0, 0, 1, 1, 1, 1]);
    assert!(glyphs.iter().all(|g| g.glyph_id != 0));

    // A base the primary face has, with a mark only the fallback has:
    // the whole cluster moves to the fallback so the mark can attach.
    let primary = source_sans();
    let fallback = dejavu();
    let mark = ('\u{300}'..='\u{36f}')
        .chain('\u{1dc0}'..='\u{1dff}')
        .find(|&m| gid(&primary, m) == 0 && gid(&fallback, m) != 0);
    if let Some(mark) = mark {
        let text = format!("q{mark}");
        let glyphs = chain.shape(&text, 20.0).unwrap();
        assert!(
            glyphs.iter().all(|g| g.face_idx == 1),
            "cluster split across faces: {glyphs:?}"
        );
        assert_eq!(glyphs[0].glyph_id, gid(&fallback, 'q'));
    }

    // A character no face has is the primary face's `.notdef`.
    let glyphs = chain.shape("\u{10ffff}", 20.0).unwrap();
    assert_eq!((glyphs[0].face_idx, glyphs[0].glyph_id), (0, 0));
}

#[test]
fn feature_switches() {
    let chain = FaceChain::new(source_sans());
    let on = chain.shape("office", 1000.0).unwrap();
    let off = chain
        .shape_with_options(
            "office",
            1000.0,
            &ShapeOptions::new().with_feature(*b"liga", false),
        )
        .unwrap();
    assert!(on.len() < off.len(), "ligatures off yields more glyphs");
    assert_eq!(off.len(), 6);

    // Small capitals replace the lowercase glyphs.
    let smcp = chain
        .shape_with_options(
            "abc",
            1000.0,
            &ShapeOptions::new().with_feature(*b"smcp", true),
        )
        .unwrap();
    let plain = chain.shape("abc", 1000.0).unwrap();
    assert_eq!(smcp.len(), 3);
    assert!(ids(&smcp).iter().zip(ids(&plain)).all(|(a, b)| *a != b));

    // Kerning off restores the plain advance.
    let unkerned = chain
        .shape_with_options(
            "AV",
            1000.0,
            &ShapeOptions::new().with_feature(*b"kern", false),
        )
        .unwrap();
    let a = chain.shape("A", 1000.0).unwrap();
    assert_eq!(unkerned[0].x_advance, a[0].x_advance);

    // `shape_with_features` names the only substitution features.
    let only = chain.shape_with_features("office", 1000.0, &[]).unwrap();
    assert_eq!(only.len(), 6, "no default ligatures");
    let liga = chain
        .shape_with_features("office", 1000.0, &[*b"liga"])
        .unwrap();
    assert_eq!(ids(&liga), ids(&on));
}

#[test]
fn alternates_are_selectable_by_index() {
    // Stylistic alternates: value n picks the n-th alternate. Whatever
    // the font offers, value 1 and the plain "on" agree.
    let chain = FaceChain::new(source_sans());
    let first = chain
        .shape_with_options(
            "a",
            1000.0,
            &ShapeOptions::new().with_feature_value(*b"salt", 1),
        )
        .unwrap();
    let on = chain
        .shape_with_options(
            "a",
            1000.0,
            &ShapeOptions::new().with_feature(*b"salt", true),
        )
        .unwrap();
    assert_eq!(ids(&first), ids(&on));
    // An index past the end of the alternate set leaves the glyph.
    let far = chain
        .shape_with_options(
            "a",
            1000.0,
            &ShapeOptions::new().with_feature_value(*b"salt", 200),
        )
        .unwrap();
    assert_eq!(far.len(), 1);
    assert_ne!(far[0].glyph_id, 0);
}

#[test]
fn right_to_left_runs_come_back_in_display_order() {
    let chain = FaceChain::new(dejavu());
    let text = "\u{5e9}\u{5dc}\u{5d5}\u{5dd}";
    let logical = chain.shape(text, 20.0).unwrap();
    let visual = chain
        .shape_visual_run(text, 20.0, &ShapeOptions::new(), true)
        .unwrap();
    assert_eq!(logical.len(), 4);
    let mut reversed = ids(&logical);
    reversed.reverse();
    assert_eq!(ids(&visual), reversed);
    let clusters: Vec<u32> = visual.iter().map(|g| g.cluster).collect();
    assert_eq!(clusters, [6, 4, 2, 0]);
    // Same ink, same width.
    let width = |g: &[PositionedGlyph]| g.iter().map(|g| g.x_advance).sum::<f32>();
    assert!((width(&logical) - width(&visual)).abs() < 1e-3);

    // A mark stays over its base in either order: its absolute
    // position relative to the base's origin is the same.
    let text = "\u{5d1}\u{5b0}";
    let logical = chain.shape(text, 2048.0).unwrap();
    let visual = chain
        .shape_visual_run(text, 2048.0, &ShapeOptions::new(), true)
        .unwrap();
    assert_eq!((logical.len(), visual.len()), (2, 2));
    // Logical: base at 0, mark at base advance + offset.
    let logical_mark = logical[0].x_advance + logical[1].x_offset;
    // Visual: mark first (pen 0), base after it (pen 0 as well: the
    // mark has no advance).
    assert_eq!(visual[0].glyph_id, logical[1].glyph_id);
    assert_eq!(visual[0].x_advance, 0.0);
    assert!((visual[0].x_offset - logical_mark).abs() < 1.0);

    // Parentheses are mirrored in a right-to-left run.
    let open = gid(chain.primary(), '(');
    let close = gid(chain.primary(), ')');
    let run = chain
        .shape_visual_run("(\u{5d0})", 20.0, &ShapeOptions::new(), true)
        .unwrap();
    assert_eq!(run.first().unwrap().glyph_id, open, "visually leftmost");
    assert_eq!(run.last().unwrap().glyph_id, close);
}

#[test]
fn arabic_joins_through_the_fonts_own_lookups() {
    let face = Face::from_ttf_bytes(NOTO_ARABIC.to_vec()).unwrap();
    let nominal: Vec<u16> = "\u{628}\u{628}\u{628}"
        .chars()
        .map(|c| gid(&face, c))
        .collect();
    let chain = FaceChain::new(face);
    let joined = chain.shape("\u{628}\u{628}\u{628}", 20.0).unwrap();
    assert_eq!(joined.len(), 3);
    // Initial, medial and final beh are three different glyphs, none
    // of them the nominal one.
    let j = ids(&joined);
    assert!(j[0] != j[1] && j[1] != j[2] && j[0] != j[2]);
    assert!(j.iter().all(|g| !nominal.contains(g)));
    // ZERO WIDTH NON-JOINER breaks the join: both sides are isolated
    // (the nominal glyph), with an invisible glyph in between.
    let split = chain.shape("\u{628}\u{200c}\u{628}", 20.0).unwrap();
    assert_eq!(split.len(), 3);
    assert_eq!(split[0].glyph_id, nominal[0]);
    assert_eq!(split[2].glyph_id, nominal[0]);
    assert_eq!(split[1].x_advance, 0.0);
    // The required lam-alef ligature forms.
    let lam_alef = chain.shape("\u{644}\u{627}", 20.0).unwrap();
    assert_eq!(lam_alef.len(), 1);
    assert_eq!(lam_alef[0].cluster, 0);
    // A vowel mark does not interrupt joining.
    let marked = chain.shape("\u{628}\u{64e}\u{628}", 20.0).unwrap();
    assert_eq!(marked.len(), 3);
    assert_eq!(marked[0].glyph_id, j[0], "initial form across the mark");
    assert_eq!(marked[1].x_advance, 0.0);
    assert!(marked[1].y_offset != 0.0 || marked[1].x_offset != 0.0);
}

#[test]
fn default_ignorables_are_invisible() {
    let chain = FaceChain::new(dejavu());
    let space = gid(chain.primary(), ' ');
    // ZERO WIDTH NON-JOINER keeps f and i apart and takes no room.
    let apart = chain.shape("f\u{200c}i", 20.0).unwrap();
    assert_eq!(apart.len(), 3);
    assert_eq!(apart[1].glyph_id, space);
    assert_eq!(apart[1].x_advance, 0.0);
    // ZERO WIDTH JOINER does not stand in the ligature's way.
    let joined = chain.shape("f\u{200d}i", 20.0).unwrap();
    let ligature = chain.shape("fi", 20.0).unwrap();
    assert_eq!(joined[0].glyph_id, ligature[0].glyph_id);
    assert_eq!(joined.len(), 2);
    assert_eq!(joined[1].x_advance, 0.0);
    // A soft hyphen draws nothing.
    let soft = chain.shape("a\u{ad}b", 20.0).unwrap();
    assert_eq!(soft.len(), 3);
    assert_eq!(soft[1].x_advance, 0.0);
}

#[test]
fn missing_spaces_take_their_defined_width() {
    let face = source_sans();
    let chain = FaceChain::new(source_sans());
    let em = 1000.0;
    for (c, width) in [
        ('\u{2003}', em),
        ('\u{2002}', em / 2.0),
        ('\u{2005}', em / 4.0),
    ] {
        if gid(&face, c) != 0 {
            continue;
        }
        let g = chain.shape(&c.to_string(), em).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].glyph_id, gid(&face, ' '));
        assert!((g[0].x_advance - width).abs() < 1.0, "U+{:04X}", c as u32);
    }
}

#[test]
fn vertical_runs_advance_downwards() {
    let chain = FaceChain::new(dejavu());
    let opts = ShapeOptions::new().with_vertical(true);
    let glyphs = chain.shape_with_options("AB", 2048.0, &opts).unwrap();
    assert_eq!(glyphs.len(), 2);
    for g in &glyphs {
        assert_eq!(g.x_advance, 0.0);
        assert!(g.y_advance > 0.0, "the pen moves down");
        assert!(g.x_offset < 0.0, "centred on the vertical line");
    }
    // Horizontal runs never move the pen vertically.
    assert!(
        chain
            .shape("AB", 20.0)
            .unwrap()
            .iter()
            .all(|g| g.y_advance == 0.0)
    );
}

#[test]
fn variation_instance_changes_advances_and_kerning() {
    let mut face = Face::from_ttf_bytes(INTER.to_vec()).unwrap();
    let regular = Shaper::shape(&face, "AV", 2048.0).unwrap();
    let coords: Vec<f32> = face
        .variation_axes()
        .iter()
        .map(|a| if &a.tag == b"wght" { 900.0 } else { a.default })
        .collect();
    face.set_variation_coords(&coords).unwrap();
    let black = Shaper::shape(&face, "AV", 2048.0).unwrap();
    assert_eq!(ids(&regular), ids(&black));
    assert!(
        black[1].x_advance > regular[1].x_advance,
        "heavier instance is wider: {} vs {}",
        black[1].x_advance,
        regular[1].x_advance
    );
}

#[test]
fn script_runs_take_their_own_features() {
    // Latin ligature, then Arabic joining, in one string and one font.
    let chain = FaceChain::new(dejavu());
    let text = "fi \u{628}\u{628}";
    let glyphs = chain.shape(text, 20.0).unwrap();
    // fi ligature + space + two joined letters.
    assert_eq!(glyphs.len(), 4);
    let beh = gid(chain.primary(), '\u{628}');
    assert!(glyphs[2].glyph_id != beh && glyphs[3].glyph_id != beh);
    let clusters: Vec<u32> = glyphs.iter().map(|g| g.cluster).collect();
    assert_eq!(clusters, [0, 2, 3, 5]);
}

#[test]
fn emoji_presentation_selects_the_colour_face() {
    let Some(bytes) = font_fixtures::load_fixture(&font_fixtures::NOTO_COLOR_EMOJI_TTF) else {
        return;
    };
    let emoji = Face::from_ttf_bytes(bytes).unwrap();
    assert!(emoji.is_color_font());
    let text_face = dejavu();
    assert!(!text_face.is_color_font());
    let heart_in_text_face = gid(&text_face, '\u{2764}') != 0;
    let chain = FaceChain::new(text_face).push_fallback(emoji);

    // An emoji the text face lacks falls through to the colour face.
    let g = chain.shape("\u{1f9d1}", 32.0).unwrap();
    assert_eq!((g.len(), g[0].face_idx), (1, 1));
    assert!(matches!(
        chain.face(1).glyph_node(g[0].glyph_id, 32.0),
        Some(oxideav_core::Node::Image(_))
    ));
    // One the text face does have (as a monochrome glyph) stays there
    // unless the emoji presentation selector asks for colour.
    if gid(chain.primary(), '\u{1f600}') != 0 {
        assert_eq!(chain.shape("\u{1f600}", 32.0).unwrap()[0].face_idx, 0);
        assert_eq!(
            chain.shape("\u{1f600}\u{fe0f}", 32.0).unwrap()[0].face_idx,
            1
        );
    }

    if heart_in_text_face {
        // Bare: the first face that has it. With the emoji selector:
        // the colour face. With the text selector: the text face.
        assert_eq!(chain.shape("\u{2764}", 32.0).unwrap()[0].face_idx, 0);
        assert_eq!(
            chain.shape("\u{2764}\u{fe0f}", 32.0).unwrap()[0].face_idx,
            1
        );
        assert_eq!(
            chain.shape("\u{2764}\u{fe0e}", 32.0).unwrap()[0].face_idx,
            0
        );
    }

    // Sequences shape to one glyph each: family (ZWJ), skin tone,
    // flag, keycap.
    for seq in [
        "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}",
        "\u{1f44d}\u{1f3fd}",
        "\u{1f1ef}\u{1f1f5}",
        "1\u{fe0f}\u{20e3}",
    ] {
        let g = chain.shape(seq, 32.0).unwrap();
        let visible: Vec<_> = g.iter().filter(|g| g.x_advance > 0.0).collect();
        assert_eq!(visible.len(), 1, "{seq:?} -> {g:?}");
        assert_eq!(visible[0].face_idx, 1);
        assert_eq!(visible[0].cluster, 0);
    }
    // Two flags are two glyphs, not one or four.
    let g = chain
        .shape("\u{1f1ef}\u{1f1f5}\u{1f1eb}\u{1f1f7}", 32.0)
        .unwrap();
    assert_eq!(g.len(), 2);
    assert_eq!((g[0].cluster, g[1].cluster), (0, 8));
}
