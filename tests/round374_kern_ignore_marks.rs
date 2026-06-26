//! Round 374 — GPOS pair kerning honours the IGNORE_MARKS LookupFlag.
//!
//! Per the §2 LookupFlag bit enumeration
//! (`docs/text/opentype/otspec-chapter2-common-layout-tables.html`,
//! IGNORE_MARKS `0x0008` — "If set, skips over all combining marks")
//! and the GPOS PairPos description
//! (`docs/text/opentype/otspec-gpos.html`), a PairPos lookup whose
//! `lookupFlag` sets IGNORE_MARKS forms each kern pair between a glyph
//! and the nearest *following non-mark* glyph. A `base + mark + base`
//! run must therefore kern base↔base across the intervening mark.
//!
//! ## Fixture
//!
//! A synthetic TTF with 4 glyphs (`.notdef`, 'A'→1, combining-mark
//! U+0301 → 2, 'V'→3), upem 1000, every glyph 500 wide. GDEF
//! classifies gid 2 as a mark (GlyphClassDef class 3). GPOS publishes
//! one PairPos Format 1 lookup with lookupFlag = IGNORE_MARKS kerning
//! the pair (A, V) by −120 design units on A's xAdvance.
//!
//! Shaping "A\u{0301}V" at 1000 px (scale 1.0):
//! - the mark gid 2 sits between 'A' and 'V';
//! - with IGNORE_MARKS honoured, the (A, V) pair kerns → A.x_advance =
//!   500 − 120 = **380**;
//! - the mark's own advance is later zeroed by mark attachment (no
//!   anchors here, so it keeps 500 — but it is not a kern member).

use oxideav_scribe::{Face, Shaper};

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

/// GDEF v1.0 with a GlyphClassDef (ClassDef Format 2) marking gid 2 as
/// class 3 (mark).
fn build_gdef() -> Vec<u8> {
    // ClassDef Format 2: classFormat(2), rangeCount(1), one ClassRangeRecord
    // (startGlyphID=2, endGlyphID=2, class=3).
    let mut class_def = Vec::new();
    class_def.extend_from_slice(&2u16.to_be_bytes()); // classFormat
    class_def.extend_from_slice(&1u16.to_be_bytes()); // classRangeCount
    class_def.extend_from_slice(&2u16.to_be_bytes()); // startGlyphID
    class_def.extend_from_slice(&2u16.to_be_bytes()); // endGlyphID
    class_def.extend_from_slice(&3u16.to_be_bytes()); // class = MARK

    let header_len: u16 = 12; // v1.0 header
    let mut gdef = Vec::new();
    gdef.extend_from_slice(&1u16.to_be_bytes()); // majorVersion
    gdef.extend_from_slice(&0u16.to_be_bytes()); // minorVersion
    gdef.extend_from_slice(&header_len.to_be_bytes()); // glyphClassDefOffset
    gdef.extend_from_slice(&0u16.to_be_bytes()); // attachListOffset
    gdef.extend_from_slice(&0u16.to_be_bytes()); // ligCaretListOffset
    gdef.extend_from_slice(&0u16.to_be_bytes()); // markAttachClassDefOffset
    gdef.extend_from_slice(&class_def);
    pad_to_4(&mut gdef);
    gdef
}

/// GPOS with one PairPos Format 1 lookup (lookupFlag set by caller)
/// kerning (gid 1, gid 3) by `kern` units on the first glyph's
/// xAdvance.
fn build_gpos_pairpos(lookup_flag: u16, kern: i16) -> Vec<u8> {
    // ----- PairValueRecord: secondGlyph(3) + valueRecord1 (xAdvance). -----
    // valueFormat1 = X_ADVANCE (0x0004), valueFormat2 = 0.
    // PairSet: pairValueCount(1) + PairValueRecord(secondGlyph, xAdv).
    let mut pair_set = Vec::new();
    pair_set.extend_from_slice(&1u16.to_be_bytes()); // pairValueCount
    pair_set.extend_from_slice(&3u16.to_be_bytes()); // secondGlyph = 'V'
    pair_set.extend_from_slice(&kern.to_be_bytes()); // valueRecord1.xAdvance

    // ----- PairPosFormat1 subtable. ---------------------------------------
    // format(2B) cov(2B) vf1(2B) vf2(2B) pairSetCount(2B) pairSetOffsets[1](2B)
    let header_len: u16 = 12;
    let cov_off: u16 = header_len;
    let cov_len: u16 = 6; // Coverage Format 1: fmt + count + 1 glyph
    let pair_set_off: u16 = cov_off + cov_len;

    let mut sub = Vec::new();
    sub.extend_from_slice(&1u16.to_be_bytes()); // posFormat = 1
    sub.extend_from_slice(&cov_off.to_be_bytes()); // coverageOffset
    sub.extend_from_slice(&0x0004u16.to_be_bytes()); // valueFormat1 = X_ADVANCE
    sub.extend_from_slice(&0u16.to_be_bytes()); // valueFormat2 = 0
    sub.extend_from_slice(&1u16.to_be_bytes()); // pairSetCount
    sub.extend_from_slice(&pair_set_off.to_be_bytes()); // pairSetOffsets[0]
                                                        // Coverage Format 1 covering gid 1 ('A').
    sub.extend_from_slice(&1u16.to_be_bytes()); // coverageFormat
    sub.extend_from_slice(&1u16.to_be_bytes()); // glyphCount
    sub.extend_from_slice(&1u16.to_be_bytes()); // glyphArray[0] = 'A'
    sub.extend_from_slice(&pair_set);

    // ----- Lookup: type 2 (PairPos), supplied flag, 1 subtable. -----------
    let mut lookup = Vec::new();
    lookup.extend_from_slice(&2u16.to_be_bytes()); // lookupType = 2
    lookup.extend_from_slice(&lookup_flag.to_be_bytes()); // lookupFlag
    lookup.extend_from_slice(&1u16.to_be_bytes()); // subTableCount
    lookup.extend_from_slice(&8u16.to_be_bytes()); // subTableOffsets[0]
    lookup.extend_from_slice(&sub);

    let mut lookup_list = Vec::new();
    lookup_list.extend_from_slice(&1u16.to_be_bytes()); // lookupCount
    lookup_list.extend_from_slice(&4u16.to_be_bytes()); // lookupOffsets[0]
    lookup_list.extend_from_slice(&lookup);

    let mut feature_record = Vec::new();
    feature_record.extend_from_slice(&0u16.to_be_bytes()); // featureParamsOffset
    feature_record.extend_from_slice(&1u16.to_be_bytes()); // lookupIndexCount
    feature_record.extend_from_slice(&0u16.to_be_bytes()); // lookupListIndices[0]

    let mut feature_list = Vec::new();
    feature_list.extend_from_slice(&1u16.to_be_bytes()); // featureCount
    feature_list.extend_from_slice(b"kern"); // featureTag
    feature_list.extend_from_slice(&8u16.to_be_bytes()); // featureOffset
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

/// Build the synthetic TTF. `gdef` toggles whether a GDEF table is
/// present (so a no-GDEF control keeps literal-adjacency kerning).
fn build_synthetic_ttf(lookup_flag: u16, kern: i16, gdef: bool) -> Vec<u8> {
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

    // cmap format 4: two segments — 'A'(0x41)→1, and a range for the
    // combining acute U+0301 → 2 and 'V'(0x56) → 3. Simplest: three
    // single-char segments via per-segment idDelta.
    // Segments (endCode order): 0x0301 → gid2, 0x0056 → gid3, 0x0041 → gid1.
    // cmap requires ascending endCode, so order: 0x0041, 0x0056, 0x0301, 0xFFFF.
    let segs: [(u16, u16, u16); 3] = [
        (0x0041, 0x0041, 1), // 'A'
        (0x0056, 0x0056, 3), // 'V'
        (0x0301, 0x0301, 2), // combining acute
    ];
    let segcount: u16 = segs.len() as u16 + 1; // + terminator 0xFFFF
    let segcountx2 = segcount * 2;
    let search_range = {
        let mut p = 1u16;
        while p * 2 <= segcount {
            p *= 2;
        }
        p * 2
    };
    let entry_selector = (search_range / 2).trailing_zeros() as u16;
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
    // endCode[]
    for &(_, end, _) in &segs {
        sub.extend_from_slice(&end.to_be_bytes());
    }
    sub.extend_from_slice(&0xFFFFu16.to_be_bytes());
    sub.extend_from_slice(&0u16.to_be_bytes()); // reservedPad
                                                // startCode[]
    for &(start, _, _) in &segs {
        sub.extend_from_slice(&start.to_be_bytes());
    }
    sub.extend_from_slice(&0xFFFFu16.to_be_bytes());
    // idDelta[] — gid - startCode (mod 0x10000)
    for &(start, _, gid) in &segs {
        let delta = gid.wrapping_sub(start);
        sub.extend_from_slice(&delta.to_be_bytes());
    }
    sub.extend_from_slice(&1u16.to_be_bytes()); // terminator idDelta
                                                // idRangeOffset[] — all 0 (direct idDelta)
    for _ in &segs {
        sub.extend_from_slice(&0u16.to_be_bytes());
    }
    sub.extend_from_slice(&0u16.to_be_bytes()); // terminator idRangeOffset
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

    // Table order must be ascending by tag: GDEF, GPOS, cmap, glyf, ...
    let mut tables_meta: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
    if gdef {
        tables_meta.push((b"GDEF", build_gdef()));
    }
    tables_meta.push((b"GPOS", build_gpos_pairpos(lookup_flag, kern)));
    tables_meta.push((b"cmap", cmap));
    tables_meta.push((b"glyf", glyf));
    tables_meta.push((b"head", head));
    tables_meta.push((b"hhea", hhea));
    tables_meta.push((b"hmtx", hmtx));
    tables_meta.push((b"loca", loca));
    tables_meta.push((b"maxp", maxp));
    tables_meta.push((b"name", name));
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

fn shape(face: &Face, text: &str, size_px: f32) -> Vec<oxideav_scribe::PositionedGlyph> {
    Shaper::shape(face, text, size_px).expect("shape")
}

const IGNORE_MARKS: u16 = 0x0008;

#[test]
fn kern_skips_intervening_mark_when_flag_set() {
    let bytes = build_synthetic_ttf(IGNORE_MARKS, -120, true);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    // "A" + combining acute + "V"
    let glyphs = shape(&face, "A\u{0301}V", 1000.0);
    assert_eq!(glyphs.len(), 3);
    assert_eq!(
        glyphs.iter().map(|g| g.glyph_id).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    // The (A, V) pair kerns across the mark: A.x_advance = 500 − 120.
    assert_eq!(glyphs[0].x_advance, 380.0);
    // Direct "AV" (no mark) kerns the same.
    let glyphs2 = shape(&face, "AV", 1000.0);
    assert_eq!(glyphs2[0].x_advance, 380.0);
}

#[test]
fn no_gdef_keeps_literal_adjacency() {
    // Without a GDEF table the IGNORE_MARKS skip predicate degenerates
    // to "never skip" (the §2 requirement), so the literal-adjacency
    // walk runs: 'A' kerns against the mark gid 2 (uncovered → 0), so
    // A.x_advance stays 500.
    let bytes = build_synthetic_ttf(IGNORE_MARKS, -120, false);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    let glyphs = shape(&face, "A\u{0301}V", 1000.0);
    assert_eq!(glyphs[0].x_advance, 500.0);
    // "AV" with the glyphs literally adjacent still kerns.
    let glyphs2 = shape(&face, "AV", 1000.0);
    assert_eq!(glyphs2[0].x_advance, 380.0);
}

#[test]
fn flag_clear_does_not_skip_marks() {
    // With the flag CLEAR (and GDEF present), the pair is literal
    // neighbours: 'A' kerns against the mark (uncovered → 0), so
    // A.x_advance stays 500 in the marked run.
    let bytes = build_synthetic_ttf(0, -120, true);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    let glyphs = shape(&face, "A\u{0301}V", 1000.0);
    assert_eq!(glyphs[0].x_advance, 500.0);
    // Adjacent "AV" still kerns regardless of the flag.
    let glyphs2 = shape(&face, "AV", 1000.0);
    assert_eq!(glyphs2[0].x_advance, 380.0);
}
