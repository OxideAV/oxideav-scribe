//! Round 276 — GPOS LookupType 3 (Cursive Attachment Positioning,
//! CursivePosFormat1) wired into the shaper's positioning pipeline.
//!
//! Per the GPOS chapter (`docs/text/opentype/otspec-gpos.html`,
//! "Lookup type 3 subtable: cursive attachment positioning"), a
//! CursivePos subtable describes how to connect adjacent glyphs by
//! aligning two anchor points: the designated **exit** point of a
//! glyph, and the designated **entry** point of the following glyph.
//! The two axes work differently:
//!
//! - **Line-layout direction** (X, for horizontal layout): "the
//!   layout engine adjusts the advance of the first glyph (in
//!   logical order)" so the anchors align in that direction.
//! - **Cross-stream direction** (Y): "placement of one glyph is
//!   adjusted to make the anchors align"; with the parent lookup's
//!   RIGHT_TO_LEFT flag clear, the **second** glyph is adjusted —
//!   the semantics the round-276 pass implements.
//!
//! Either anchor offset may be NULL, "in which case no positioning
//! adjustment is applied".
//!
//! ## Why a synthetic fixture
//!
//! None of the vendored fixtures (DejaVu Sans, Inter Variable,
//! Source Sans 3) publish a LookupType-3 GPOS lookup, so — mirroring
//! the round-125 / round-128 approach — we build a minimal synthetic
//! TTF in-test: 4 glyphs (`.notdef` + 'a' + 'b' + 'c'), a Format-4
//! cmap, and a GPOS table publishing one `curs` feature under script
//! `DFLT` wrapping a single CursivePosFormat1 lookup. Every byte
//! layout follows the OpenType spec chapters staged under
//! `docs/text/opentype/` (GPOS header / Coverage / Anchor Format 1 /
//! CursivePosFormat1 + EntryExit record tables).
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
//! Shaping "abc" at 1000 px (scale 1.0) must therefore produce:
//!
//! - 'a'.x_advance = 450 − 50 = **400** (advance of the *first*
//!   glyph rewritten);
//! - 'b'.y_offset = −(100 − 300) = **+200** raster px (entry sits
//!   300 above b's origin but must land only 100 above the baseline
//!   → b moves *down*; raster Y grows downward);
//! - 'b'.x_advance = 450 − 0 = **450**;
//! - 'c'.y_offset = 200 − (−200 − 0) = **+400** — the cross-stream
//!   adjustment accumulates down the connected chain.

use oxideav_scribe::{Face, Shaper};

// ---------------------------------------------------------------------------
// Synthetic-font builder
// ---------------------------------------------------------------------------

type Anchor = Option<(i16, i16)>;

fn pad_to_4(buf: &mut Vec<u8>) {
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

/// Sum a byte slice as big-endian u32 words for the sfnt table-record
/// checksum field (zero-padded to a 4-byte boundary per the spec).
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

/// Build the GPOS table: ScriptList(DFLT) + FeatureList(`curs`) +
/// LookupList with one LookupType-3 lookup whose CursivePosFormat1
/// subtable covers gids `1..=anchors.len()` with the supplied
/// per-glyph `(entry, exit)` anchor pairs.
fn build_gpos_cursive(anchors: &[(Anchor, Anchor)]) -> Vec<u8> {
    let n = anchors.len() as u16;

    // ----- CursivePosFormat1 subtable -------------------------------------
    // uint16 posFormat = 1
    // Offset16 coverageOffset            (from subtable start)
    // uint16 entryExitCount
    // EntryExit entryExitRecords[count]  (2 Offset16 each, NULL = 0)
    // ... Coverage table, then Anchor tables (Format 1: 6 bytes each).
    let header_len: u16 = 6 + 4 * n;
    let cov_off: u16 = header_len;
    let cov_len: u16 = 4 + 2 * n; // Coverage Format 1: fmt + count + glyphs
    let anchors_base: u16 = cov_off + cov_len;

    // Lay out anchor tables in declaration order, recording offsets.
    let mut anchor_bytes = Vec::new();
    let mut records: Vec<(u16, u16)> = Vec::new(); // (entryOff, exitOff)
    for &(entry, exit) in anchors {
        let mut place = |a: Anchor| -> u16 {
            match a {
                None => 0, // NULL offset — no anchor
                Some((x, y)) => {
                    let off = anchors_base + anchor_bytes.len() as u16;
                    anchor_bytes.extend_from_slice(&1u16.to_be_bytes()); // anchorFormat 1
                    anchor_bytes.extend_from_slice(&x.to_be_bytes()); // xCoordinate
                    anchor_bytes.extend_from_slice(&y.to_be_bytes()); // yCoordinate
                    off
                }
            }
        };
        let e = place(entry);
        let x = place(exit);
        records.push((e, x));
    }

    let mut sub = Vec::new();
    sub.extend_from_slice(&1u16.to_be_bytes()); // posFormat = 1
    sub.extend_from_slice(&cov_off.to_be_bytes());
    sub.extend_from_slice(&n.to_be_bytes()); // entryExitCount
    for (e, x) in &records {
        sub.extend_from_slice(&e.to_be_bytes());
        sub.extend_from_slice(&x.to_be_bytes());
    }
    // Coverage Format 1 covering gids 1..=n in ascending order.
    sub.extend_from_slice(&1u16.to_be_bytes()); // coverageFormat
    sub.extend_from_slice(&n.to_be_bytes()); // glyphCount
    for gid in 1..=n {
        sub.extend_from_slice(&gid.to_be_bytes());
    }
    sub.extend_from_slice(&anchor_bytes);

    // ----- Lookup: type 3, flag 0 (RIGHT_TO_LEFT clear), 1 subtable. ------
    let mut lookup = Vec::new();
    lookup.extend_from_slice(&3u16.to_be_bytes()); // lookupType = 3 (CursivePos)
    lookup.extend_from_slice(&0u16.to_be_bytes()); // lookupFlag
    lookup.extend_from_slice(&1u16.to_be_bytes()); // subTableCount
    lookup.extend_from_slice(&8u16.to_be_bytes()); // subTableOffsets[0]
    lookup.extend_from_slice(&sub);

    let mut lookup_list = Vec::new();
    lookup_list.extend_from_slice(&1u16.to_be_bytes()); // lookupCount
    lookup_list.extend_from_slice(&4u16.to_be_bytes()); // lookupOffsets[0]
    lookup_list.extend_from_slice(&lookup);

    // ----- Feature `curs` with one lookup (index 0). ----------------------
    let mut feature_record = Vec::new();
    feature_record.extend_from_slice(&0u16.to_be_bytes()); // featureParamsOffset
    feature_record.extend_from_slice(&1u16.to_be_bytes()); // lookupIndexCount
    feature_record.extend_from_slice(&0u16.to_be_bytes()); // lookupListIndices[0]

    let mut feature_list = Vec::new();
    feature_list.extend_from_slice(&1u16.to_be_bytes()); // featureCount
    feature_list.extend_from_slice(b"curs"); // featureTag
    feature_list.extend_from_slice(&8u16.to_be_bytes()); // featureOffset (2 + 6)
    feature_list.extend_from_slice(&feature_record);

    // ----- LangSys / Script / ScriptList (DFLT default LangSys). ----------
    let mut langsys = Vec::new();
    langsys.extend_from_slice(&0u16.to_be_bytes()); // lookupOrderOffset (reserved)
    langsys.extend_from_slice(&0xFFFFu16.to_be_bytes()); // requiredFeatureIndex
    langsys.extend_from_slice(&1u16.to_be_bytes()); // featureIndexCount
    langsys.extend_from_slice(&0u16.to_be_bytes()); // featureIndices[0]

    let mut script = Vec::new();
    script.extend_from_slice(&4u16.to_be_bytes()); // defaultLangSysOffset
    script.extend_from_slice(&0u16.to_be_bytes()); // langSysCount
    script.extend_from_slice(&langsys);

    let mut script_list = Vec::new();
    script_list.extend_from_slice(&1u16.to_be_bytes()); // scriptCount
    script_list.extend_from_slice(b"DFLT"); // scriptTag
    script_list.extend_from_slice(&8u16.to_be_bytes()); // scriptOffset (2 + 6)
    script_list.extend_from_slice(&script);

    // ----- GPOS header (version 1.0). --------------------------------------
    let header_len: u16 = 10;
    let script_list_off: u16 = header_len;
    let feature_list_off: u16 = script_list_off + script_list.len() as u16;
    let lookup_list_off: u16 = feature_list_off + feature_list.len() as u16;

    let mut gpos = Vec::new();
    gpos.extend_from_slice(&1u16.to_be_bytes()); // majorVersion
    gpos.extend_from_slice(&0u16.to_be_bytes()); // minorVersion
    gpos.extend_from_slice(&script_list_off.to_be_bytes());
    gpos.extend_from_slice(&feature_list_off.to_be_bytes());
    gpos.extend_from_slice(&lookup_list_off.to_be_bytes());
    gpos.extend_from_slice(&script_list);
    gpos.extend_from_slice(&feature_list);
    gpos.extend_from_slice(&lookup_list);
    pad_to_4(&mut gpos);
    gpos
}

/// Build a minimal synthetic TTF: 4 glyphs (`.notdef` + 'a' + 'b' +
/// 'c' → gids 0..=3), upem 1000, every glyph 500 units wide, plus —
/// when `cursive` is `Some` — a GPOS table from [`build_gpos_cursive`]
/// whose per-glyph anchors apply to gids 1..=3 in order.
fn build_synthetic_ttf(cursive: Option<&[(Anchor, Anchor)]>) -> Vec<u8> {
    // ----- glyf: empty (all glyphs zero-length via all-zero loca). --------
    let glyf: Vec<u8> = Vec::new();

    // ----- loca: 5 short offsets, all zero. --------------------------------
    let mut loca = Vec::new();
    for _ in 0..5 {
        loca.extend_from_slice(&0u16.to_be_bytes());
    }
    pad_to_4(&mut loca);

    // ----- maxp 0.5 (TrueType). --------------------------------------------
    let mut maxp = Vec::new();
    maxp.extend_from_slice(&0x00005000u32.to_be_bytes()); // version 0.5
    maxp.extend_from_slice(&4u16.to_be_bytes()); // numGlyphs
    pad_to_4(&mut maxp);

    // ----- head --------------------------------------------------------------
    let mut head = Vec::new();
    head.extend_from_slice(&0x00010000u32.to_be_bytes()); // version 1.0
    head.extend_from_slice(&0x00010000u32.to_be_bytes()); // fontRevision
    head.extend_from_slice(&0u32.to_be_bytes()); // checkSumAdjustment
    head.extend_from_slice(&0x5F0F3CF5u32.to_be_bytes()); // magicNumber
    head.extend_from_slice(&0u16.to_be_bytes()); // flags
    head.extend_from_slice(&1000u16.to_be_bytes()); // unitsPerEm
    head.extend_from_slice(&0i64.to_be_bytes()); // created
    head.extend_from_slice(&0i64.to_be_bytes()); // modified
    head.extend_from_slice(&0i16.to_be_bytes()); // xMin
    head.extend_from_slice(&0i16.to_be_bytes()); // yMin
    head.extend_from_slice(&0i16.to_be_bytes()); // xMax
    head.extend_from_slice(&0i16.to_be_bytes()); // yMax
    head.extend_from_slice(&0u16.to_be_bytes()); // macStyle
    head.extend_from_slice(&8u16.to_be_bytes()); // lowestRecPPEM
    head.extend_from_slice(&2i16.to_be_bytes()); // fontDirectionHint
    head.extend_from_slice(&0i16.to_be_bytes()); // indexToLocFormat (short)
    head.extend_from_slice(&0i16.to_be_bytes()); // glyphDataFormat
    pad_to_4(&mut head);

    // ----- hhea --------------------------------------------------------------
    let mut hhea = Vec::new();
    hhea.extend_from_slice(&0x00010000u32.to_be_bytes()); // version 1.0
    hhea.extend_from_slice(&800i16.to_be_bytes()); // ascender
    hhea.extend_from_slice(&(-200i16).to_be_bytes()); // descender
    hhea.extend_from_slice(&0i16.to_be_bytes()); // lineGap
    hhea.extend_from_slice(&1000u16.to_be_bytes()); // advanceWidthMax
    hhea.extend_from_slice(&0i16.to_be_bytes()); // minLeftSideBearing
    hhea.extend_from_slice(&0i16.to_be_bytes()); // minRightSideBearing
    hhea.extend_from_slice(&1000i16.to_be_bytes()); // xMaxExtent
    hhea.extend_from_slice(&1i16.to_be_bytes()); // caretSlopeRise
    hhea.extend_from_slice(&0i16.to_be_bytes()); // caretSlopeRun
    hhea.extend_from_slice(&0i16.to_be_bytes()); // caretOffset
    for _ in 0..4 {
        hhea.extend_from_slice(&0i16.to_be_bytes()); // reserved
    }
    hhea.extend_from_slice(&0i16.to_be_bytes()); // metricDataFormat
    hhea.extend_from_slice(&4u16.to_be_bytes()); // numberOfHMetrics
    pad_to_4(&mut hhea);

    // ----- hmtx: 4 longHorMetric records. ------------------------------------
    let mut hmtx = Vec::new();
    for _ in 0..4 {
        hmtx.extend_from_slice(&500u16.to_be_bytes()); // advanceWidth
        hmtx.extend_from_slice(&0i16.to_be_bytes()); // lsb
    }
    pad_to_4(&mut hmtx);

    // ----- name: zero records. -----------------------------------------------
    let mut name = Vec::new();
    name.extend_from_slice(&0u16.to_be_bytes()); // format
    name.extend_from_slice(&0u16.to_be_bytes()); // count
    name.extend_from_slice(&6u16.to_be_bytes()); // stringOffset
    pad_to_4(&mut name);

    // ----- cmap: format 4, one segment 'a'..'c' → gids 1..3. -----------------
    let segcount: u16 = 2;
    let segcountx2 = segcount * 2;
    let search_range = 2u16;
    let entry_selector = 0u16;
    let range_shift = segcountx2.wrapping_sub(search_range);
    let mut sub = Vec::new();
    sub.extend_from_slice(&4u16.to_be_bytes()); // format
    let length_offset = sub.len();
    sub.extend_from_slice(&0u16.to_be_bytes()); // length (patched)
    sub.extend_from_slice(&0u16.to_be_bytes()); // language
    sub.extend_from_slice(&segcountx2.to_be_bytes());
    sub.extend_from_slice(&search_range.to_be_bytes());
    sub.extend_from_slice(&entry_selector.to_be_bytes());
    sub.extend_from_slice(&range_shift.to_be_bytes());
    sub.extend_from_slice(&0x0063u16.to_be_bytes()); // endCode[0] = 'c'
    sub.extend_from_slice(&0xFFFFu16.to_be_bytes()); // endCode[1]
    sub.extend_from_slice(&0u16.to_be_bytes()); // reservedPad
    sub.extend_from_slice(&0x0061u16.to_be_bytes()); // startCode[0] = 'a'
    sub.extend_from_slice(&0xFFFFu16.to_be_bytes()); // startCode[1]
    sub.extend_from_slice(&0xFFA0u16.to_be_bytes()); // idDelta[0] = 1 - 0x61
    sub.extend_from_slice(&1u16.to_be_bytes()); // idDelta[1]
    sub.extend_from_slice(&0u16.to_be_bytes()); // idRangeOffset[0]
    sub.extend_from_slice(&0u16.to_be_bytes()); // idRangeOffset[1]
    let sub_len = sub.len() as u16;
    sub[length_offset..length_offset + 2].copy_from_slice(&sub_len.to_be_bytes());

    let mut cmap = Vec::new();
    cmap.extend_from_slice(&0u16.to_be_bytes()); // version
    cmap.extend_from_slice(&1u16.to_be_bytes()); // numTables
    cmap.extend_from_slice(&3u16.to_be_bytes()); // platformID (Microsoft)
    cmap.extend_from_slice(&1u16.to_be_bytes()); // encodingID (Unicode BMP)
    cmap.extend_from_slice(&12u32.to_be_bytes()); // subtable offset (4 + 8)
    cmap.extend_from_slice(&sub);
    pad_to_4(&mut cmap);

    // ----- sfnt assembly (tags in ascending order). ---------------------------
    let mut tables_meta: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
    if let Some(anchors) = cursive {
        tables_meta.push((b"GPOS", build_gpos_cursive(anchors)));
    }
    tables_meta.push((b"cmap", cmap));
    tables_meta.push((b"glyf", glyf));
    tables_meta.push((b"head", head));
    tables_meta.push((b"hhea", hhea));
    tables_meta.push((b"hmtx", hmtx));
    tables_meta.push((b"loca", loca));
    tables_meta.push((b"maxp", maxp));
    tables_meta.push((b"name", name));
    let num_tables = tables_meta.len() as u16;

    let log2 = 15 - num_tables.leading_zeros() as u16; // floor(log2(numTables))
    let search_range = 16u16 * (1u16 << log2);
    let entry_selector = log2;
    let range_shift = num_tables * 16 - search_range;

    let header_size = 12usize;
    let record_size = 16usize;
    let body_start = header_size + record_size * tables_meta.len();

    let mut out = Vec::new();
    out.extend_from_slice(&0x00010000u32.to_be_bytes()); // sfnt scaler
    out.extend_from_slice(&num_tables.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&range_shift.to_be_bytes());
    out.resize(body_start, 0);

    let mut records: Vec<(&[u8; 4], u32, u32, u32)> = Vec::new(); // tag, ck, off, len
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

/// The anchor topology from the module docs: a→exit only, b→both,
/// c→entry only.
fn test_anchors() -> Vec<(Anchor, Anchor)> {
    vec![
        (None, Some((450, 100))),             // gid 1, 'a'
        (Some((50, 300)), Some((450, -200))), // gid 2, 'b'
        (Some((0, 0)), None),                 // gid 3, 'c'
    ]
}

fn shape(face: &Face, text: &str, size_px: f32) -> Vec<oxideav_scribe::PositionedGlyph> {
    Shaper::shape(face, text, size_px).expect("shape")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn cursive_chain_aligns_anchors_and_cascades_cross_stream() {
    let bytes = build_synthetic_ttf(Some(&test_anchors()));
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    // size 1000 px on a upem-1000 font → scale 1.0, anchors map 1:1.
    let glyphs = shape(&face, "abc", 1000.0);
    assert_eq!(glyphs.len(), 3);
    assert_eq!(
        glyphs.iter().map(|g| g.glyph_id).collect::<Vec<_>>(),
        [1, 2, 3]
    );

    // Pair (a, b): exit (450, 100) ← entry (50, 300).
    // Line-layout direction: the FIRST glyph's advance is rewritten
    // to 450 − 50 = 400.
    assert_eq!(glyphs[0].x_advance, 400.0);
    assert_eq!(glyphs[0].y_offset, 0.0); // first glyph never moves (flag clear)
                                         // Cross-stream: the SECOND glyph moves down 200 raster px so its
                                         // entry (300 above its origin) lands at the exit height (100).
    assert_eq!(glyphs[1].y_offset, 200.0);

    // Pair (b, c): exit (450, −200) ← entry (0, 0).
    assert_eq!(glyphs[1].x_advance, 450.0);
    // Chain accumulation: 200 − (−200 − 0) = 400.
    assert_eq!(glyphs[2].y_offset, 400.0);
    // Nothing follows 'c' — its advance is untouched.
    assert_eq!(glyphs[2].x_advance, 500.0);
}

#[test]
fn cursive_adjustments_scale_with_size() {
    let bytes = build_synthetic_ttf(Some(&test_anchors()));
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    // 500 px on upem 1000 → scale 0.5; every adjustment halves.
    let glyphs = shape(&face, "abc", 500.0);
    assert_eq!(glyphs[0].x_advance, 200.0);
    assert_eq!(glyphs[1].y_offset, 100.0);
    assert_eq!(glyphs[1].x_advance, 225.0);
    assert_eq!(glyphs[2].y_offset, 200.0);
    assert_eq!(glyphs[2].x_advance, 250.0);
}

#[test]
fn null_anchor_offsets_apply_no_adjustment() {
    let bytes = build_synthetic_ttf(Some(&test_anchors()));
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");

    // "aa": the second 'a' has no ENTRY anchor → pair skipped.
    let glyphs = shape(&face, "aa", 1000.0);
    assert_eq!(glyphs[0].x_advance, 500.0);
    assert_eq!(glyphs[1].y_offset, 0.0);

    // "ca": 'c' has no EXIT anchor → pair skipped.
    let glyphs = shape(&face, "ca", 1000.0);
    assert_eq!(glyphs[0].x_advance, 500.0);
    assert_eq!(glyphs[1].y_offset, 0.0);

    // "ab": both anchors present → first glyph adjusted; the second
    // glyph's own advance stays at its hmtx value (nothing follows).
    let glyphs = shape(&face, "ab", 1000.0);
    assert_eq!(glyphs[0].x_advance, 400.0);
    assert_eq!(glyphs[1].y_offset, 200.0);
    assert_eq!(glyphs[1].x_advance, 500.0);
}

#[test]
fn font_without_gpos_is_a_no_op() {
    let bytes = build_synthetic_ttf(None);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    let glyphs = shape(&face, "abc", 1000.0);
    for g in &glyphs {
        assert_eq!(g.x_advance, 500.0);
        assert_eq!(g.x_offset, 0.0);
        assert_eq!(g.y_offset, 0.0);
    }
}
