//! Round 374 — GPOS LookupType 3 (Cursive Attachment) with the
//! parent lookup's **RIGHT_TO_LEFT** flag (`LookupFlag` bit `0x0001`)
//! set.
//!
//! Per the GPOS chapter (`docs/text/opentype/otspec-gpos.html`,
//! cursive attachment cross-stream note) and the common-table
//! LookupFlag bit enumeration
//! (`docs/text/opentype/otspec-chapter2-common-layout-tables.html`,
//! RIGHT_TO_LEFT `0x0001`):
//!
//! > For the cross-stream direction, placement of one glyph is
//! > adjusted to make the anchors align. Which glyph is adjusted is
//! > determined by the RIGHT_TO_LEFT flag in the parent lookup table:
//! > if the RIGHT_TO_LEFT flag is clear, the second glyph is adjusted
//! > to align anchors with the first glyph; if the RIGHT_TO_LEFT flag
//! > is set, the first glyph is adjusted to align anchors with the
//! > second glyph.
//! >
//! > Note that, if the RIGHT_TO_LEFT lookup flag is set, then the last
//! > glyph in the connected sequence keeps its initial position in the
//! > cross-stream direction relative to the baseline, and the
//! > cross-stream positions of the preceding, connected glyphs are
//! > adjusted.
//!
//! The line-layout (X advance) handling is identical for both flag
//! states ("the layout engine adjusts the advance of the first glyph
//! (in logical order)"), so only the cross-stream (Y) cascade
//! direction differs.
//!
//! ## Anchor topology under test
//!
//! upem = 1000, every glyph advances 500. Anchors (font units, Y-up):
//!
//! | gid | char | entry      | exit        |
//! |-----|------|------------|-------------|
//! | 1   | 'a'  | —          | (450, 100)  |
//! | 2   | 'b'  | (50, 300)  | (450, -200) |
//! | 3   | 'c'  | (0, 0)     | —           |
//!
//! Shaping "abc" at 1000 px (scale 1.0) with RIGHT_TO_LEFT **set**:
//!
//! - X advances are unchanged from the flag-clear case: a→400, b→450.
//! - Cross-stream: the LAST connected glyph ('c') holds its initial
//!   position (`y_offset = 0`). Resolving backward, each first glyph is
//!   pinned to the (already-resolved) second by
//!   `first.y_offset = second.y_offset + (exit_y − entry_y)·scale`:
//!     * pair (b, c): exit (450, −200) ← entry (0, 0) →
//!       `b.y_offset = 0 + (−200 − 0) = −200`.
//!     * pair (a, b): exit (450, 100) ← entry (50, 300) →
//!       `a.y_offset = −200 + (100 − 300) = −400`.

use oxideav_scribe::{Face, Shaper};

type Anchor = Option<(i16, i16)>;

fn pad_to_4(buf: &mut Vec<u8>) {
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

fn table_checksum(data: &[u8]) -> u32 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 4 <= data.len() {
        sum = sum.wrapping_add(u32::from_be_bytes([
            data[i],
            data[i + 1],
            data[i + 2],
            data[i + 3],
        ]));
        i += 4;
    }
    if i < data.len() {
        let mut last = [0u8; 4];
        last[..data.len() - i].copy_from_slice(&data[i..]);
        sum = sum.wrapping_add(u32::from_be_bytes(last));
    }
    sum
}

/// Build the GPOS table exactly as the round-276 helper does, but with
/// the lookup's `lookupFlag` set to `lookup_flag` (so the same anchor
/// topology can be exercised with RIGHT_TO_LEFT `0x0001`).
fn build_gpos_cursive(anchors: &[(Anchor, Anchor)], lookup_flag: u16) -> Vec<u8> {
    let n = anchors.len() as u16;

    let header_len: u16 = 6 + 4 * n;
    let cov_off: u16 = header_len;
    let cov_len: u16 = 4 + 2 * n;
    let anchors_base: u16 = cov_off + cov_len;

    let mut anchor_bytes = Vec::new();
    let mut records: Vec<(u16, u16)> = Vec::new();
    for &(entry, exit) in anchors {
        let mut place = |a: Anchor| -> u16 {
            match a {
                None => 0,
                Some((x, y)) => {
                    let off = anchors_base + anchor_bytes.len() as u16;
                    anchor_bytes.extend_from_slice(&1u16.to_be_bytes());
                    anchor_bytes.extend_from_slice(&x.to_be_bytes());
                    anchor_bytes.extend_from_slice(&y.to_be_bytes());
                    off
                }
            }
        };
        let e = place(entry);
        let x = place(exit);
        records.push((e, x));
    }

    let mut sub = Vec::new();
    sub.extend_from_slice(&1u16.to_be_bytes());
    sub.extend_from_slice(&cov_off.to_be_bytes());
    sub.extend_from_slice(&n.to_be_bytes());
    for (e, x) in &records {
        sub.extend_from_slice(&e.to_be_bytes());
        sub.extend_from_slice(&x.to_be_bytes());
    }
    sub.extend_from_slice(&1u16.to_be_bytes());
    sub.extend_from_slice(&n.to_be_bytes());
    for gid in 1..=n {
        sub.extend_from_slice(&gid.to_be_bytes());
    }
    sub.extend_from_slice(&anchor_bytes);

    let mut lookup = Vec::new();
    lookup.extend_from_slice(&3u16.to_be_bytes()); // lookupType = 3
    lookup.extend_from_slice(&lookup_flag.to_be_bytes()); // lookupFlag
    lookup.extend_from_slice(&1u16.to_be_bytes()); // subTableCount
    lookup.extend_from_slice(&8u16.to_be_bytes()); // subTableOffsets[0]
    lookup.extend_from_slice(&sub);

    let mut lookup_list = Vec::new();
    lookup_list.extend_from_slice(&1u16.to_be_bytes());
    lookup_list.extend_from_slice(&4u16.to_be_bytes());
    lookup_list.extend_from_slice(&lookup);

    let mut feature_record = Vec::new();
    feature_record.extend_from_slice(&0u16.to_be_bytes());
    feature_record.extend_from_slice(&1u16.to_be_bytes());
    feature_record.extend_from_slice(&0u16.to_be_bytes());

    let mut feature_list = Vec::new();
    feature_list.extend_from_slice(&1u16.to_be_bytes());
    feature_list.extend_from_slice(b"curs");
    feature_list.extend_from_slice(&8u16.to_be_bytes());
    feature_list.extend_from_slice(&feature_record);

    let mut langsys = Vec::new();
    langsys.extend_from_slice(&0u16.to_be_bytes());
    langsys.extend_from_slice(&0xFFFFu16.to_be_bytes());
    langsys.extend_from_slice(&1u16.to_be_bytes());
    langsys.extend_from_slice(&0u16.to_be_bytes());

    let mut script = Vec::new();
    script.extend_from_slice(&4u16.to_be_bytes());
    script.extend_from_slice(&0u16.to_be_bytes());
    script.extend_from_slice(&langsys);

    let mut script_list = Vec::new();
    script_list.extend_from_slice(&1u16.to_be_bytes());
    script_list.extend_from_slice(b"DFLT");
    script_list.extend_from_slice(&8u16.to_be_bytes());
    script_list.extend_from_slice(&script);

    let header_len: u16 = 10;
    let script_list_off: u16 = header_len;
    let feature_list_off: u16 = script_list_off + script_list.len() as u16;
    let lookup_list_off: u16 = feature_list_off + feature_list.len() as u16;

    let mut gpos = Vec::new();
    gpos.extend_from_slice(&1u16.to_be_bytes());
    gpos.extend_from_slice(&0u16.to_be_bytes());
    gpos.extend_from_slice(&script_list_off.to_be_bytes());
    gpos.extend_from_slice(&feature_list_off.to_be_bytes());
    gpos.extend_from_slice(&lookup_list_off.to_be_bytes());
    gpos.extend_from_slice(&script_list);
    gpos.extend_from_slice(&feature_list);
    gpos.extend_from_slice(&lookup_list);
    pad_to_4(&mut gpos);
    gpos
}

fn build_synthetic_ttf(anchors: &[(Anchor, Anchor)], lookup_flag: u16) -> Vec<u8> {
    let glyf: Vec<u8> = Vec::new();

    let mut loca = Vec::new();
    for _ in 0..5 {
        loca.extend_from_slice(&0u16.to_be_bytes());
    }
    pad_to_4(&mut loca);

    let mut maxp = Vec::new();
    maxp.extend_from_slice(&0x00005000u32.to_be_bytes());
    maxp.extend_from_slice(&4u16.to_be_bytes());
    pad_to_4(&mut maxp);

    let mut head = Vec::new();
    head.extend_from_slice(&0x00010000u32.to_be_bytes());
    head.extend_from_slice(&0x00010000u32.to_be_bytes());
    head.extend_from_slice(&0u32.to_be_bytes());
    head.extend_from_slice(&0x5F0F3CF5u32.to_be_bytes());
    head.extend_from_slice(&0u16.to_be_bytes());
    head.extend_from_slice(&1000u16.to_be_bytes());
    head.extend_from_slice(&0i64.to_be_bytes());
    head.extend_from_slice(&0i64.to_be_bytes());
    head.extend_from_slice(&0i16.to_be_bytes());
    head.extend_from_slice(&0i16.to_be_bytes());
    head.extend_from_slice(&0i16.to_be_bytes());
    head.extend_from_slice(&0i16.to_be_bytes());
    head.extend_from_slice(&0u16.to_be_bytes());
    head.extend_from_slice(&8u16.to_be_bytes());
    head.extend_from_slice(&2i16.to_be_bytes());
    head.extend_from_slice(&0i16.to_be_bytes());
    head.extend_from_slice(&0i16.to_be_bytes());
    pad_to_4(&mut head);

    let mut hhea = Vec::new();
    hhea.extend_from_slice(&0x00010000u32.to_be_bytes());
    hhea.extend_from_slice(&800i16.to_be_bytes());
    hhea.extend_from_slice(&(-200i16).to_be_bytes());
    hhea.extend_from_slice(&0i16.to_be_bytes());
    hhea.extend_from_slice(&1000u16.to_be_bytes());
    hhea.extend_from_slice(&0i16.to_be_bytes());
    hhea.extend_from_slice(&0i16.to_be_bytes());
    hhea.extend_from_slice(&1000i16.to_be_bytes());
    hhea.extend_from_slice(&1i16.to_be_bytes());
    hhea.extend_from_slice(&0i16.to_be_bytes());
    hhea.extend_from_slice(&0i16.to_be_bytes());
    for _ in 0..4 {
        hhea.extend_from_slice(&0i16.to_be_bytes());
    }
    hhea.extend_from_slice(&0i16.to_be_bytes());
    hhea.extend_from_slice(&4u16.to_be_bytes());
    pad_to_4(&mut hhea);

    let mut hmtx = Vec::new();
    for _ in 0..4 {
        hmtx.extend_from_slice(&500u16.to_be_bytes());
        hmtx.extend_from_slice(&0i16.to_be_bytes());
    }
    pad_to_4(&mut hmtx);

    let mut name = Vec::new();
    name.extend_from_slice(&0u16.to_be_bytes());
    name.extend_from_slice(&0u16.to_be_bytes());
    name.extend_from_slice(&6u16.to_be_bytes());
    pad_to_4(&mut name);

    let segcount: u16 = 2;
    let segcountx2 = segcount * 2;
    let search_range = 2u16;
    let entry_selector = 0u16;
    let range_shift = segcountx2.wrapping_sub(search_range);
    let mut sub = Vec::new();
    sub.extend_from_slice(&4u16.to_be_bytes());
    let length_offset = sub.len();
    sub.extend_from_slice(&0u16.to_be_bytes());
    sub.extend_from_slice(&0u16.to_be_bytes());
    sub.extend_from_slice(&segcountx2.to_be_bytes());
    sub.extend_from_slice(&search_range.to_be_bytes());
    sub.extend_from_slice(&entry_selector.to_be_bytes());
    sub.extend_from_slice(&range_shift.to_be_bytes());
    sub.extend_from_slice(&0x0063u16.to_be_bytes());
    sub.extend_from_slice(&0xFFFFu16.to_be_bytes());
    sub.extend_from_slice(&0u16.to_be_bytes());
    sub.extend_from_slice(&0x0061u16.to_be_bytes());
    sub.extend_from_slice(&0xFFFFu16.to_be_bytes());
    sub.extend_from_slice(&0xFFA0u16.to_be_bytes());
    sub.extend_from_slice(&1u16.to_be_bytes());
    sub.extend_from_slice(&0u16.to_be_bytes());
    sub.extend_from_slice(&0u16.to_be_bytes());
    let sub_len = sub.len() as u16;
    sub[length_offset..length_offset + 2].copy_from_slice(&sub_len.to_be_bytes());

    let mut cmap = Vec::new();
    cmap.extend_from_slice(&0u16.to_be_bytes());
    cmap.extend_from_slice(&1u16.to_be_bytes());
    cmap.extend_from_slice(&3u16.to_be_bytes());
    cmap.extend_from_slice(&1u16.to_be_bytes());
    cmap.extend_from_slice(&12u32.to_be_bytes());
    cmap.extend_from_slice(&sub);
    pad_to_4(&mut cmap);

    let tables_meta: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"GPOS", build_gpos_cursive(anchors, lookup_flag)),
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"name", name),
    ];
    let num_tables = tables_meta.len() as u16;

    let log2 = 15 - num_tables.leading_zeros() as u16;
    let search_range = 16u16 * (1u16 << log2);
    let entry_selector = log2;
    let range_shift = num_tables * 16 - search_range;

    let header_size = 12usize;
    let record_size = 16usize;
    let body_start = header_size + record_size * tables_meta.len();

    let mut out = Vec::new();
    out.extend_from_slice(&0x00010000u32.to_be_bytes());
    out.extend_from_slice(&num_tables.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&range_shift.to_be_bytes());
    out.resize(body_start, 0);

    let mut records: Vec<(&[u8; 4], u32, u32, u32)> = Vec::new();
    for (tag, data) in &tables_meta {
        let offset = out.len() as u32;
        records.push((tag, table_checksum(data), offset, data.len() as u32));
        out.extend_from_slice(data);
        pad_to_4(&mut out);
    }
    for (i, (tag, checksum, offset, length)) in records.iter().enumerate() {
        let rec_pos = header_size + i * record_size;
        out[rec_pos..rec_pos + 4].copy_from_slice(&tag[..]);
        out[rec_pos + 4..rec_pos + 8].copy_from_slice(&checksum.to_be_bytes());
        out[rec_pos + 8..rec_pos + 12].copy_from_slice(&offset.to_be_bytes());
        out[rec_pos + 12..rec_pos + 16].copy_from_slice(&length.to_be_bytes());
    }
    out
}

fn test_anchors() -> Vec<(Anchor, Anchor)> {
    vec![
        (None, Some((450, 100))),
        (Some((50, 300)), Some((450, -200))),
        (Some((0, 0)), None),
    ]
}

fn shape(face: &Face, text: &str, size_px: f32) -> Vec<oxideav_scribe::PositionedGlyph> {
    Shaper::shape(face, text, size_px).expect("shape")
}

const RIGHT_TO_LEFT: u16 = 0x0001;

#[test]
fn rtl_flag_anchors_chain_to_last_glyph() {
    let bytes = build_synthetic_ttf(&test_anchors(), RIGHT_TO_LEFT);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    let glyphs = shape(&face, "abc", 1000.0);
    assert_eq!(glyphs.len(), 3);
    assert_eq!(
        glyphs.iter().map(|g| g.glyph_id).collect::<Vec<_>>(),
        [1, 2, 3]
    );

    // X (line-layout) advances are identical to the flag-clear case.
    assert_eq!(glyphs[0].x_advance, 400.0);
    assert_eq!(glyphs[1].x_advance, 450.0);
    assert_eq!(glyphs[2].x_advance, 500.0);

    // Cross-stream: the LAST glyph 'c' holds its position; preceding
    // connected glyphs are adjusted backward.
    assert_eq!(glyphs[2].y_offset, 0.0);
    assert_eq!(glyphs[1].y_offset, -200.0);
    assert_eq!(glyphs[0].y_offset, -400.0);
}

#[test]
fn rtl_flag_scales_with_size() {
    let bytes = build_synthetic_ttf(&test_anchors(), RIGHT_TO_LEFT);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    // 500 px on upem 1000 → scale 0.5; every adjustment halves.
    let glyphs = shape(&face, "abc", 500.0);
    assert_eq!(glyphs[0].x_advance, 200.0);
    assert_eq!(glyphs[1].x_advance, 225.0);
    assert_eq!(glyphs[2].y_offset, 0.0);
    assert_eq!(glyphs[1].y_offset, -100.0);
    assert_eq!(glyphs[0].y_offset, -200.0);
}

#[test]
fn flag_clear_keeps_forward_cascade() {
    // Sanity guard: with the flag CLEAR the same topology produces the
    // round-276 forward cascade (first glyph holds its Y at 0).
    let bytes = build_synthetic_ttf(&test_anchors(), 0);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    let glyphs = shape(&face, "abc", 1000.0);
    assert_eq!(glyphs[0].y_offset, 0.0);
    assert_eq!(glyphs[1].y_offset, 200.0);
    assert_eq!(glyphs[2].y_offset, 400.0);
}

#[test]
fn rtl_pair_with_null_anchor_is_skipped() {
    // "ca": 'c' has no EXIT anchor → the (c, a) pair is not connected,
    // so neither glyph is cross-stream adjusted even under RTL.
    let bytes = build_synthetic_ttf(&test_anchors(), RIGHT_TO_LEFT);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    let glyphs = shape(&face, "ca", 1000.0);
    assert_eq!(glyphs[0].x_advance, 500.0);
    assert_eq!(glyphs[0].y_offset, 0.0);
    assert_eq!(glyphs[1].y_offset, 0.0);

    // "ab": both anchors present → 'a' adjusted to align with 'b'
    // (which holds position as the chain's last glyph).
    let glyphs = shape(&face, "ab", 1000.0);
    assert_eq!(glyphs[0].x_advance, 400.0);
    assert_eq!(glyphs[1].y_offset, 0.0);
    assert_eq!(glyphs[0].y_offset, -200.0);
}
