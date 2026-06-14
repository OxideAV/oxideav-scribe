//! Round 298 — GPOS LookupType 1 (Single Adjustment Positioning,
//! SinglePos) wired into the shaper's positioning pipeline.
//!
//! Per the GPOS chapter (`docs/text/opentype/otspec-gpos.html`,
//! "Lookup Type 1: Single Adjustment Positioning Subtable"), a
//! SinglePos subtable "is used to adjust the placement or advance of a
//! single glyph, such as a subscript or superscript. In addition, a
//! SinglePos subtable is commonly used to implement lookup data for
//! contextual positioning." Two sub-table formats exist:
//!
//! - **SinglePosFormat1** "applies the same positioning value or
//!   values to each of the Coverage Index glyphs" — a single shared
//!   `ValueRecord`.
//! - **SinglePosFormat2** "provides an array of ValueRecords that
//!   contains one positioning value for each glyph in the Coverage
//!   table" — a per-glyph `ValueRecord`.
//!
//! The four `ValueRecord` geometric fields map onto a positioned
//! glyph as: `xPlacement` shifts the drawn position right (added to
//! `x_offset`), `yPlacement` shifts it up in TT Y-up space (subtracted
//! from `y_offset`, which is raster Y-down), and `xAdvance` widens or
//! narrows the horizontal advance. `yAdvance` only affects
//! vertical-layout runs and is ignored on the horizontal pen.
//!
//! ## Why a synthetic fixture
//!
//! None of the vendored fixtures (DejaVu Sans, Inter Variable, Source
//! Sans 3) publish a LookupType-1 GPOS lookup in a place this shaper
//! reaches, so — mirroring the round-125 / round-128 / round-276
//! approach — we build a minimal synthetic TTF in-test: 4 glyphs
//! (`.notdef` + 'a' + 'b' + 'c'), a Format-4 cmap, and a GPOS table
//! publishing one `kern`-style feature under script `DFLT` wrapping a
//! single SinglePos lookup. Every byte layout follows the OpenType
//! spec chapters staged under `docs/text/opentype/` (GPOS header /
//! Coverage / ValueRecord / SinglePosFormat1 + Format2 tables).

use oxideav_scribe::{Face, Shaper};

// ---------------------------------------------------------------------------
// Synthetic-font builder
// ---------------------------------------------------------------------------

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

/// One `ValueRecord`'s four geometric fields (font units, Y-up): we
/// always emit all four (valueFormat = 0x000F) so the parser exercises
/// the full record decode.
#[derive(Clone, Copy)]
struct Value {
    x_placement: i16,
    y_placement: i16,
    x_advance: i16,
    y_advance: i16,
}

const VALUE_FORMAT_ALL: u16 = 0x000F; // X/Y placement + X/Y advance

fn push_value(buf: &mut Vec<u8>, v: Value) {
    buf.extend_from_slice(&v.x_placement.to_be_bytes());
    buf.extend_from_slice(&v.y_placement.to_be_bytes());
    buf.extend_from_slice(&v.x_advance.to_be_bytes());
    buf.extend_from_slice(&v.y_advance.to_be_bytes());
}

/// One SinglePos sub-table, either Format 1 (shared value) or Format 2
/// (per-glyph values), covering gids `1..=n` in ascending order.
enum SinglePos {
    Format1 { value: Value },
    Format2 { values: Vec<Value> },
}

fn build_single_pos_subtable(sp: &SinglePos, n: u16) -> Vec<u8> {
    match sp {
        SinglePos::Format1 { value } => {
            // posFormat(2) coverageOffset(2) valueFormat(2) valueRecord(8)
            let header_len: u16 = 2 + 2 + 2 + 8;
            let cov_off = header_len;
            let mut sub = Vec::new();
            sub.extend_from_slice(&1u16.to_be_bytes()); // posFormat = 1
            sub.extend_from_slice(&cov_off.to_be_bytes());
            sub.extend_from_slice(&VALUE_FORMAT_ALL.to_be_bytes());
            push_value(&mut sub, *value);
            // Coverage Format 1, gids 1..=n.
            sub.extend_from_slice(&1u16.to_be_bytes()); // coverageFormat
            sub.extend_from_slice(&n.to_be_bytes()); // glyphCount
            for gid in 1..=n {
                sub.extend_from_slice(&gid.to_be_bytes());
            }
            sub
        }
        SinglePos::Format2 { values } => {
            let count = values.len() as u16;
            // posFormat(2) coverageOffset(2) valueFormat(2) valueCount(2)
            // valueRecords(8*count)
            let header_len: u16 = 2 + 2 + 2 + 2 + 8 * count;
            let cov_off = header_len;
            let mut sub = Vec::new();
            sub.extend_from_slice(&2u16.to_be_bytes()); // posFormat = 2
            sub.extend_from_slice(&cov_off.to_be_bytes());
            sub.extend_from_slice(&VALUE_FORMAT_ALL.to_be_bytes());
            sub.extend_from_slice(&count.to_be_bytes()); // valueCount
            for v in values {
                push_value(&mut sub, *v);
            }
            sub.extend_from_slice(&1u16.to_be_bytes()); // coverageFormat
            sub.extend_from_slice(&count.to_be_bytes()); // glyphCount
            for gid in 1..=count {
                sub.extend_from_slice(&gid.to_be_bytes());
            }
            sub
        }
    }
}

/// Build the GPOS table: ScriptList(DFLT) + FeatureList(`kern`) +
/// LookupList with one LookupType-1 lookup wrapping `sp`.
fn build_gpos_single_pos(sp: &SinglePos, n: u16) -> Vec<u8> {
    let sub = build_single_pos_subtable(sp, n);

    // ----- Lookup: type 1, flag 0, 1 subtable. ----------------------------
    let mut lookup = Vec::new();
    lookup.extend_from_slice(&1u16.to_be_bytes()); // lookupType = 1 (SinglePos)
    lookup.extend_from_slice(&0u16.to_be_bytes()); // lookupFlag
    lookup.extend_from_slice(&1u16.to_be_bytes()); // subTableCount
    lookup.extend_from_slice(&8u16.to_be_bytes()); // subTableOffsets[0]
    lookup.extend_from_slice(&sub);

    let mut lookup_list = Vec::new();
    lookup_list.extend_from_slice(&1u16.to_be_bytes()); // lookupCount
    lookup_list.extend_from_slice(&4u16.to_be_bytes()); // lookupOffsets[0]
    lookup_list.extend_from_slice(&lookup);

    // ----- Feature `kern` with one lookup (index 0). ----------------------
    let mut feature_record = Vec::new();
    feature_record.extend_from_slice(&0u16.to_be_bytes()); // featureParamsOffset
    feature_record.extend_from_slice(&1u16.to_be_bytes()); // lookupIndexCount
    feature_record.extend_from_slice(&0u16.to_be_bytes()); // lookupListIndices[0]

    let mut feature_list = Vec::new();
    feature_list.extend_from_slice(&1u16.to_be_bytes()); // featureCount
    feature_list.extend_from_slice(b"kern"); // featureTag
    feature_list.extend_from_slice(&8u16.to_be_bytes()); // featureOffset (2 + 6)
    feature_list.extend_from_slice(&feature_record);

    // ----- LangSys / Script / ScriptList (DFLT default LangSys). ----------
    let mut langsys = Vec::new();
    langsys.extend_from_slice(&0u16.to_be_bytes()); // lookupOrderOffset
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
/// when `single_pos` is `Some` — a GPOS table from
/// [`build_gpos_single_pos`].
fn build_synthetic_ttf(single_pos: Option<&SinglePos>) -> Vec<u8> {
    let glyf: Vec<u8> = Vec::new();

    let mut loca = Vec::new();
    for _ in 0..5 {
        loca.extend_from_slice(&0u16.to_be_bytes());
    }
    pad_to_4(&mut loca);

    let mut maxp = Vec::new();
    maxp.extend_from_slice(&0x00005000u32.to_be_bytes()); // version 0.5
    maxp.extend_from_slice(&4u16.to_be_bytes()); // numGlyphs
    pad_to_4(&mut maxp);

    let mut head = Vec::new();
    head.extend_from_slice(&0x00010000u32.to_be_bytes()); // version
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

    let mut hhea = Vec::new();
    hhea.extend_from_slice(&0x00010000u32.to_be_bytes()); // version
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

    let mut hmtx = Vec::new();
    for _ in 0..4 {
        hmtx.extend_from_slice(&500u16.to_be_bytes()); // advanceWidth
        hmtx.extend_from_slice(&0i16.to_be_bytes()); // lsb
    }
    pad_to_4(&mut hmtx);

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
    cmap.extend_from_slice(&12u32.to_be_bytes()); // subtable offset
    cmap.extend_from_slice(&sub);
    pad_to_4(&mut cmap);

    let mut tables_meta: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
    if let Some(sp) = single_pos {
        tables_meta.push((b"GPOS", build_gpos_single_pos(sp, 3)));
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

    let log2 = 15 - num_tables.leading_zeros() as u16;
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn format1_shared_value_applies_to_every_covered_glyph() {
    // One shared ValueRecord: shift each glyph right 30, up 80, and
    // widen the advance by 40 font units. upem 1000, size 1000 px →
    // scale 1.0, so font units map 1:1 onto raster px.
    let sp = SinglePos::Format1 {
        value: Value {
            x_placement: 30,
            y_placement: 80,
            x_advance: 40,
            y_advance: 0,
        },
    };
    let bytes = build_synthetic_ttf(Some(&sp));
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    let glyphs = shape(&face, "abc", 1000.0);
    assert_eq!(glyphs.len(), 3);

    for g in &glyphs {
        assert_eq!(g.x_offset, 30.0, "xPlacement → x_offset");
        // yPlacement is TT Y-up; y_offset is raster Y-down, so +80
        // up-shift becomes −80.
        assert_eq!(g.y_offset, -80.0, "yPlacement → −y_offset");
        // base hmtx advance 500 + xAdvance 40.
        assert_eq!(g.x_advance, 540.0, "xAdvance widens the advance");
    }
}

#[test]
fn format2_per_glyph_values_apply_independently() {
    // Distinct ValueRecord per covered glyph (gids 1/2/3 = a/b/c).
    let sp = SinglePos::Format2 {
        values: vec![
            Value {
                x_placement: 10,
                y_placement: 0,
                x_advance: 0,
                y_advance: 0,
            }, // 'a'
            Value {
                x_placement: 0,
                y_placement: 50,
                x_advance: -20,
                y_advance: 0,
            }, // 'b'
            Value {
                x_placement: -5,
                y_placement: 0,
                x_advance: 100,
                y_advance: 0,
            }, // 'c'
        ],
    };
    let bytes = build_synthetic_ttf(Some(&sp));
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    let glyphs = shape(&face, "abc", 1000.0);
    assert_eq!(glyphs.len(), 3);

    // 'a': x_offset +10, advance unchanged.
    assert_eq!(glyphs[0].x_offset, 10.0);
    assert_eq!(glyphs[0].y_offset, 0.0);
    assert_eq!(glyphs[0].x_advance, 500.0);

    // 'b': raised 50 (→ −50 raster), advance narrowed 20.
    assert_eq!(glyphs[1].x_offset, 0.0);
    assert_eq!(glyphs[1].y_offset, -50.0);
    assert_eq!(glyphs[1].x_advance, 480.0);

    // 'c': nudged left 5, advance widened 100.
    assert_eq!(glyphs[2].x_offset, -5.0);
    assert_eq!(glyphs[2].y_offset, 0.0);
    assert_eq!(glyphs[2].x_advance, 600.0);
}

#[test]
fn single_pos_scales_with_size() {
    // 500 px on upem 1000 → scale 0.5; every adjustment halves.
    let sp = SinglePos::Format1 {
        value: Value {
            x_placement: 30,
            y_placement: 80,
            x_advance: 40,
            y_advance: 0,
        },
    };
    let bytes = build_synthetic_ttf(Some(&sp));
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    let glyphs = shape(&face, "a", 500.0);
    assert_eq!(glyphs[0].x_offset, 15.0);
    assert_eq!(glyphs[0].y_offset, -40.0);
    // base advance 250 (500 * 0.5) + 20 (40 * 0.5).
    assert_eq!(glyphs[0].x_advance, 270.0);
}

#[test]
fn single_pos_stacks_with_kerning_offset() {
    // SinglePos x_offset must add to (not replace) the kern x_offset
    // the previous step computed. Our synthetic font ships no kern
    // pairs, so the SinglePos x_offset is the only contributor — but
    // the additive `+=` contract is what this test pins via the second
    // glyph carrying the same x_offset the first does.
    let sp = SinglePos::Format1 {
        value: Value {
            x_placement: 12,
            y_placement: 0,
            x_advance: 0,
            y_advance: 0,
        },
    };
    let bytes = build_synthetic_ttf(Some(&sp));
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    let glyphs = shape(&face, "ab", 1000.0);
    assert_eq!(glyphs[0].x_offset, 12.0);
    assert_eq!(glyphs[1].x_offset, 12.0);
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

#[test]
fn uncovered_glyph_is_untouched() {
    // Format 2 covering only gid 1 ('a'): 'b' and 'c' fall outside the
    // coverage and must pass through unchanged.
    let sp = SinglePos::Format2 {
        values: vec![Value {
            x_placement: 99,
            y_placement: 0,
            x_advance: 0,
            y_advance: 0,
        }],
    };
    let bytes = build_synthetic_ttf(Some(&sp));
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    let glyphs = shape(&face, "abc", 1000.0);
    assert_eq!(glyphs[0].x_offset, 99.0); // 'a' covered
    assert_eq!(glyphs[1].x_offset, 0.0); // 'b' uncovered
    assert_eq!(glyphs[2].x_offset, 0.0); // 'c' uncovered
}
