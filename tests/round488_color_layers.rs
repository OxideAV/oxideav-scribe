//! Layered colour glyphs (`COLR` version 0 + `CPAL`) through
//! `Face::glyph_node` / `Face::color_glyph_node`.
//!
//! No in-tree fixture ships a `COLR` table, so the tests graft one
//! onto DejaVu Sans Mono: the font's tables are re-packed behind a new
//! table directory with a hand-built `COLR` (version 0) and `CPAL`
//! (version 0) added. Layouts follow the staged references:
//! table directory — `docs/text/opentype/otspec-otff.html`; `COLR`
//! header / BaseGlyph / Layer records —
//! `docs/text/opentype/otspec-colr-v1-paint-graph.md` §2–§3; `CPAL`
//! header and BGRA colour records — `docs/text/opentype/otspec-cpal.html`.

use oxideav_core::{Node, Paint, PathCommand};
use oxideav_scribe::{ColorGlyphLayer, Face, LayerColor, OutlineCache};
use std::sync::Arc;

const DEJAVU_MONO: &[u8] = include_bytes!("fixtures/DejaVuSansMono.ttf");

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Re-pack `font` with `extra` tables added (replacing same-tag ones).
fn with_tables(font: &[u8], extra: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let num_tables = be16(font, 4) as usize;
    let mut tables: Vec<([u8; 4], Vec<u8>)> = (0..num_tables)
        .map(|i| {
            let rec = 12 + i * 16;
            let tag = [font[rec], font[rec + 1], font[rec + 2], font[rec + 3]];
            let offset = be32(font, rec + 8) as usize;
            let length = be32(font, rec + 12) as usize;
            (tag, font[offset..offset + length].to_vec())
        })
        .filter(|(tag, _)| !extra.iter().any(|(t, _)| t == tag))
        .collect();
    tables.extend(extra.iter().cloned());
    // Table records are sorted by tag.
    tables.sort_by_key(|table| table.0);

    let n = tables.len() as u16;
    let entry_selector = 15 - n.leading_zeros() as u16; // floor(log2 n)
    let search_range = (1u16 << entry_selector) * 16;
    let mut out = Vec::new();
    out.extend_from_slice(&font[0..4]); // sfntVersion
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&(n * 16 - search_range).to_be_bytes());
    let mut offset = 12 + tables.len() * 16;
    let mut body = Vec::new();
    for (tag, data) in &tables {
        // Checksum: the table as big-endian u32 words, zero padded.
        let mut padded = data.clone();
        padded.resize(data.len().next_multiple_of(4), 0);
        let checksum = padded
            .chunks(4)
            .fold(0u32, |sum, w| sum.wrapping_add(be32(w, 0)));
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum.to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        // Tables begin on four-byte boundaries.
        offset += padded.len();
        body.extend_from_slice(&padded);
    }
    out.extend_from_slice(&body);
    out
}

/// `COLR` version 0: `bases` = (base glyph, first layer, layer count),
/// sorted by glyph id; `layers` = (layer glyph, palette entry).
fn colr_v0(bases: &[(u16, u16, u16)], layers: &[(u16, u16)]) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(&0u16.to_be_bytes()); // version
    t.extend_from_slice(&(bases.len() as u16).to_be_bytes());
    t.extend_from_slice(&14u32.to_be_bytes()); // baseGlyphRecordsOffset
    t.extend_from_slice(&(14 + bases.len() as u32 * 6).to_be_bytes()); // layerRecordsOffset
    t.extend_from_slice(&(layers.len() as u16).to_be_bytes());
    for &(gid, first, count) in bases {
        for v in [gid, first, count] {
            t.extend_from_slice(&v.to_be_bytes());
        }
    }
    for &(gid, entry) in layers {
        for v in [gid, entry] {
            t.extend_from_slice(&v.to_be_bytes());
        }
    }
    t
}

/// `CPAL` version 0 with every palette `entries` RGBA colours long.
fn cpal_v0(palettes: &[&[[u8; 4]]]) -> Vec<u8> {
    let entries = palettes[0].len() as u16;
    let mut t = Vec::new();
    t.extend_from_slice(&0u16.to_be_bytes()); // version
    t.extend_from_slice(&entries.to_be_bytes()); // numPaletteEntries
    t.extend_from_slice(&(palettes.len() as u16).to_be_bytes());
    t.extend_from_slice(&(entries * palettes.len() as u16).to_be_bytes()); // numColorRecords
    t.extend_from_slice(&(12 + 2 * palettes.len() as u32).to_be_bytes()); // colorRecordsArrayOffset
    for i in 0..palettes.len() as u16 {
        t.extend_from_slice(&(i * entries).to_be_bytes()); // colorRecordIndices
    }
    for palette in palettes {
        for &[r, g, b, a] in *palette {
            t.extend_from_slice(&[b, g, r, a]); // ColorRecord is BGRA
        }
    }
    t
}

const RED: [u8; 4] = [220, 20, 30, 255];
const GREEN: [u8; 4] = [10, 200, 40, 128];
const BLUE: [u8; 4] = [5, 15, 240, 255];
const GOLD: [u8; 4] = [250, 200, 0, 255];

struct Fixture {
    face: Face,
    plain: Face,
    a: u16,
    b: u16,
    o: u16,
    x: u16,
    dot: u16,
    space: u16,
}

fn gid(face: &Face, ch: char) -> u16 {
    face.with_font(|f| f.glyph_index(ch)).unwrap().unwrap()
}

/// DejaVu Sans Mono where 'A' is [O red/blue, x green/gold, . foreground]
/// and 'B' is [x palette entry 1, space (no outline), O entry 9 (absent)].
fn fixture() -> Fixture {
    let plain = Face::from_ttf_bytes(DEJAVU_MONO.to_vec()).unwrap();
    let (a, b, o, x, dot, space) = (
        gid(&plain, 'A'),
        gid(&plain, 'B'),
        gid(&plain, 'O'),
        gid(&plain, 'x'),
        gid(&plain, '.'),
        gid(&plain, ' '),
    );
    assert!(a < b);
    let colr = colr_v0(
        &[(a, 0, 3), (b, 3, 3)],
        &[(o, 0), (x, 1), (dot, 0xFFFF), (x, 1), (space, 0), (o, 9)],
    );
    let cpal = cpal_v0(&[&[RED, GREEN], &[BLUE, GOLD]]);
    let bytes = with_tables(DEJAVU_MONO, &[(*b"COLR", colr), (*b"CPAL", cpal)]);
    let face = Face::from_ttf_bytes(bytes).expect("re-packed font parses");
    Fixture {
        face,
        plain,
        a,
        b,
        o,
        x,
        dot,
        space,
    }
}

fn commands(node: &Node) -> Vec<PathCommand> {
    match node {
        Node::Path(p) => p.path.commands.clone(),
        other => panic!("expected a path node, got {other:?}"),
    }
}

fn fill(node: &Node) -> [u8; 4] {
    match node {
        Node::Path(p) => match p.fill {
            Some(Paint::Solid(c)) => [c.r, c.g, c.b, c.a],
            ref other => panic!("expected a solid fill, got {other:?}"),
        },
        other => panic!("expected a path node, got {other:?}"),
    }
}

fn children(node: Option<Node>) -> Vec<Node> {
    match node {
        Some(Node::Group(g)) => g.children,
        other => panic!("expected a layer group, got {other:?}"),
    }
}

#[test]
fn repacked_font_is_the_same_font_without_colour_tables() {
    // The re-pack itself (no extra tables) changes nothing observable.
    let repacked = Face::from_ttf_bytes(with_tables(DEJAVU_MONO, &[])).unwrap();
    let plain = Face::from_ttf_bytes(DEJAVU_MONO.to_vec()).unwrap();
    assert!(!repacked.has_color_layers() && !plain.has_color_layers());
    assert_eq!(plain.color_palette_count(), 0);
    for ch in "AOx. g@".chars() {
        let g = gid(&plain, ch);
        assert_eq!(gid(&repacked, ch), g);
        assert_eq!(
            format!("{:?}", repacked.glyph_node(g, 20.0)),
            format!("{:?}", plain.glyph_node(g, 20.0))
        );
        assert!(plain.color_glyph_layers(g, 0).is_empty());
        assert!(plain.color_glyph_node(g, 20.0, 0, [0; 4]).is_none());
    }
}

#[test]
fn layers_resolve_bottom_first_with_palette_and_foreground() {
    let f = fixture();
    assert!(f.face.has_color_layers());
    assert_eq!(f.face.color_palette_count(), 2);

    let layer = |glyph_id, color| (glyph_id, color);
    let got: Vec<(u16, LayerColor)> = f
        .face
        .color_glyph_layers(f.a, 0)
        .iter()
        .map(|l: &ColorGlyphLayer| (l.glyph_id, l.color))
        .collect();
    assert_eq!(
        got,
        vec![
            layer(f.o, LayerColor::Palette(RED)),
            layer(f.x, LayerColor::Palette(GREEN)),
            layer(f.dot, LayerColor::Foreground),
        ]
    );
    // The second palette recolours the same layers.
    let second: Vec<LayerColor> = f
        .face
        .color_glyph_layers(f.a, 1)
        .iter()
        .map(|l| l.color)
        .collect();
    assert_eq!(
        second,
        vec![
            LayerColor::Palette(BLUE),
            LayerColor::Palette(GOLD),
            LayerColor::Foreground
        ]
    );
    // A palette that does not exist leaves only the foreground layer.
    let missing = f.face.color_glyph_layers(f.a, 7);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].color, LayerColor::Foreground);
    // 'B': palette entry 9 does not exist, so that layer is left out.
    let b: Vec<u16> = f
        .face
        .color_glyph_layers(f.b, 0)
        .iter()
        .map(|l| l.glyph_id)
        .collect();
    assert_eq!(b, vec![f.x, f.space]);
    // Layer-only and ordinary glyphs are not colour glyphs.
    for g in [f.o, f.x, f.dot, f.space, 0, u16::MAX] {
        assert!(f.face.color_glyph_layers(g, 0).is_empty());
    }
}

#[test]
fn glyph_node_is_a_group_of_filled_layer_outlines() {
    let f = fixture();
    let size = 24.0;
    let kids = children(f.face.glyph_node(f.a, size));
    assert_eq!(kids.len(), 3);
    // Each layer is exactly the layer glyph's own outline node…
    for (kid, layer_gid) in kids.iter().zip([f.o, f.x, f.dot]) {
        let own = f.plain.glyph_node(layer_gid, size).unwrap();
        assert_eq!(commands(kid), commands(&own));
    }
    // …filled from the default palette, black for the foreground.
    assert_eq!(fill(&kids[0]), RED);
    assert_eq!(fill(&kids[1]), GREEN);
    assert_eq!(fill(&kids[2]), [0, 0, 0, 255]);

    // Explicit palette + foreground.
    let kids = children(f.face.color_glyph_node(f.a, size, 1, [1, 2, 3, 4]));
    assert_eq!(
        kids.iter().map(fill).collect::<Vec<_>>(),
        vec![BLUE, GOLD, [1, 2, 3, 4]]
    );

    // 'B': the outline-less layer (space) and the unpaintable one are
    // dropped; one layer is left.
    let kids = children(f.face.glyph_node(f.b, size));
    assert_eq!(kids.len(), 1);
    assert_eq!(fill(&kids[0]), GREEN);
    assert_eq!(
        commands(&kids[0]),
        commands(&f.plain.glyph_node(f.x, size).unwrap())
    );

    // Non-colour glyphs are untouched by the colour tables.
    for g in [f.o, f.x, f.dot] {
        assert_eq!(
            format!("{:?}", f.face.glyph_node(g, size)),
            format!("{:?}", f.plain.glyph_node(g, size))
        );
    }
    assert!(f.face.glyph_node(f.space, size).is_none());
    // glyph_path stays the base glyph's own outline.
    assert_eq!(
        f.face.glyph_path(f.a).map(|p| p.commands),
        f.plain.glyph_path(f.a).map(|p| p.commands)
    );
    // Degenerate sizes.
    assert!(f.face.color_glyph_node(f.a, 0.0, 0, [0; 4]).is_none());
    assert!(f.face.color_glyph_node(f.a, f32::NAN, 0, [0; 4]).is_none());
    assert!(f.face.glyph_node(f.a, -1.0).is_none());
}

#[test]
fn layer_outlines_go_through_the_outline_cache() {
    let f = fixture();
    let want = format!("{:?}", f.face.glyph_node(f.a, 18.0));
    let cache = Arc::new(OutlineCache::default());
    let mut cached = fixture().face;
    cached.set_outline_cache(Some(cache.clone()));
    for _ in 0..3 {
        assert_eq!(format!("{:?}", cached.glyph_node(f.a, 18.0)), want);
    }
    let s = cache.stats();
    // Three layer outlines decoded once, then reused twice each.
    assert_eq!((s.misses, s.hits), (3, 6));
    // The same layer glyph drawn on its own shares the entry.
    cached.glyph_node(f.o, 30.0);
    assert_eq!(cache.stats().misses, 3);
}

/// A base glyph with no paintable layer falls back to its outline, and
/// malformed colour tables never panic.
#[test]
fn unpaintable_and_malformed_colour_tables_fall_back() {
    let plain = Face::from_ttf_bytes(DEJAVU_MONO.to_vec()).unwrap();
    let (a, space) = (gid(&plain, 'A'), gid(&plain, ' '));
    let want = format!("{:?}", plain.glyph_node(a, 16.0));

    // Every layer is outline-less or uses an absent palette entry.
    let colr = colr_v0(&[(a, 0, 2)], &[(space, 0), (a, 5)]);
    let cpal = cpal_v0(&[&[RED]]);
    let face = Face::from_ttf_bytes(with_tables(
        DEJAVU_MONO,
        &[(*b"COLR", colr.clone()), (*b"CPAL", cpal.clone())],
    ))
    .unwrap();
    assert!(face.color_glyph_node(a, 16.0, 0, [0; 4]).is_none());
    assert_eq!(format!("{:?}", face.glyph_node(a, 16.0)), want);

    // Truncations and garbage: whatever the parser makes of them, no
    // panic, and a glyph still draws as a group or as its outline.
    let good_colr = colr_v0(&[(a, 0, 1)], &[(a, 0)]);
    let mut variants: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for cut in 0..good_colr.len() {
        variants.push((good_colr[..cut].to_vec(), cpal.clone()));
    }
    for cut in 0..cpal.len() {
        variants.push((good_colr.clone(), cpal[..cut].to_vec()));
    }
    // Layer run pointing far past the layer array; huge counts.
    variants.push((colr_v0(&[(a, 40_000, 60_000)], &[(a, 0)]), cpal.clone()));
    variants.push((colr_v0(&[(a, 0, 1)], &[(u16::MAX, 0)]), cpal.clone()));
    let mut lying = good_colr.clone();
    lying[2..4].copy_from_slice(&u16::MAX.to_be_bytes()); // numBaseGlyphRecords
    variants.push((lying, cpal.clone()));
    for (colr, cpal) in variants {
        let bytes = with_tables(DEJAVU_MONO, &[(*b"COLR", colr), (*b"CPAL", cpal)]);
        let Ok(face) = Face::from_ttf_bytes(bytes) else {
            continue;
        };
        let _ = face.color_palette_count();
        let _ = face.color_glyph_layers(a, 0);
        match face.glyph_node(a, 16.0) {
            Some(Node::Group(g)) => assert!(!g.children.is_empty()),
            other => assert_eq!(format!("{other:?}"), want),
        }
    }
}
