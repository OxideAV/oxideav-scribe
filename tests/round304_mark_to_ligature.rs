//! Round 304 — GPOS LookupType 5 (Mark-to-Ligature Attachment
//! Positioning, MarkLigPosFormat1) wired into the shaper's
//! positioning pipeline.
//!
//! Per the GPOS chapter (`docs/text/opentype/otspec-gpos.html`,
//! "Mark-to-Ligature Attachment Positioning Format 1"), a MarkLigPos
//! subtable positions combining marks with respect to **ligature**
//! base glyphs. Unlike mark-to-base, a ligature carries "multiple
//! components (in a virtual sense — not actual glyphs), and each
//! component has a separate set of attachment points defined for the
//! different mark classes". The spec is explicit that the
//! component a mark associates with "is dependent on the original
//! character string and subsequent ... glyph-substitution operations,
//! not the font data alone", so the text-layout client must keep
//! track of mark-to-component associations across the GSUB ligature
//! collapse.
//!
//! The shaper recovers that association from its own step-2 ligature
//! pass: each output glyph records how many input glyphs collapsed
//! into it (the ligature's component count). A mark walked back to a
//! multi-component ligature base is assigned to a component by its
//! ordinal among the marks trailing that base, and lands on that
//! component's per-class anchor. NULL component anchors
//! (`lookup_mark_to_ligature` returns `None`) are skipped by walking
//! the remaining components.
//!
//! ## Why a synthetic fixture
//!
//! None of the vendored fixtures (DejaVu Sans, Inter Variable, Source
//! Sans 3) publish a LookupType-5 GPOS lookup, so — mirroring the
//! round-128 (GSUB ligature) and round-276 (GPOS cursive) approach —
//! we build a minimal synthetic TTF in-test. Every byte layout
//! follows the OpenType spec chapters staged under
//! `docs/text/opentype/` (GPOS header / Coverage Format 1 / Anchor
//! Format 1 / MarkArray / MarkLigPosFormat1 + LigatureArray /
//! LigatureAttach / ComponentRecord; GSUB header + LigatureSubstFormat1;
//! GDEF GlyphClassDef Format 2). No external font-shaping or
//! font-parsing library was consulted; no WebSearch / WebFetch
//! invoked.
//!
//! ## Glyph + anchor topology under test
//!
//! upem = 1000, every glyph advances 500. Glyphs:
//!
//! | gid | char         | role                          |
//! |-----|--------------|-------------------------------|
//! | 0   | —            | `.notdef`                     |
//! | 1   | 'f' U+0066   | ligature component 0          |
//! | 2   | 'i' U+0069   | ligature component 1          |
//! | 3   | (none)       | fi-ligature (GSUB type 4)     |
//! | 4   | '́' U+0301   | combining acute (GDEF mark)   |
//!
//! GSUB `liga`: `f` + `i` → gid 3 (a 2-component ligature).
//!
//! GPOS MarkLigPos (mark class 0, the acute): the fi-ligature
//! publishes two components, each with a distinct class-0 anchor:
//!   - component 0 anchor = (120, 700)
//!   - component 1 anchor = (380, 720)
//!
//! The acute mark's own anchor is (0, 0), so the applied delta equals
//! the component anchor. Shaping "fi" + acute at 1000 px (scale 1.0):
//! the ligature forms (1 base glyph), and the trailing acute attaches
//! to component **1** (its ordinal among trailing marks is 0, but it
//! defaults to the last component → component 1) landing at
//! x_offset = 380, y_offset = −720.

use oxideav_scribe::{Face, PositionedGlyph, Shaper};

// ---------------------------------------------------------------------------
// sfnt helpers (shared shape with round-128 / round-276 builders)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// GSUB: one LookupType-4 (Ligature) lookup, `f` + `i` → `lig_gid`.
// ---------------------------------------------------------------------------

/// Build a GSUB table publishing a `liga` feature under script `DFLT`
/// whose single LookupType-4 sub-table collapses `[first, second]`
/// into `lig_gid` (a 2-component ligature).
fn build_gsub_ligature(first: u16, second: u16, lig_gid: u16) -> Vec<u8> {
    // LigatureSet for `first`: one Ligature record.
    //   Ligature: ligGlyph + componentCount(=2) + componentGlyphIDs[1]
    let mut lig_body = Vec::new();
    lig_body.extend_from_slice(&lig_gid.to_be_bytes());
    lig_body.extend_from_slice(&2u16.to_be_bytes()); // componentCount
    lig_body.extend_from_slice(&second.to_be_bytes()); // components[1..]
    let lig_set_header = 4u16; // ligatureCount(2) + ligatureOffsets[1](2)
    let mut lig_set = Vec::new();
    lig_set.extend_from_slice(&1u16.to_be_bytes()); // ligatureCount
    lig_set.extend_from_slice(&lig_set_header.to_be_bytes()); // offset[0]
    lig_set.extend_from_slice(&lig_body);

    // LigatureSubstFormat1: substFormat + coverageOffset +
    // ligatureSetCount + ligatureSetOffsets[1] + Coverage + LigatureSet.
    let sub_header_len = 8u16; // 2+2+2+2
    let cov_off = sub_header_len;
    let mut coverage = Vec::new();
    coverage.extend_from_slice(&1u16.to_be_bytes()); // coverageFormat 1
    coverage.extend_from_slice(&1u16.to_be_bytes()); // glyphCount
    coverage.extend_from_slice(&first.to_be_bytes());
    let lig_set_off = cov_off + coverage.len() as u16;

    let mut sub_table = Vec::new();
    sub_table.extend_from_slice(&1u16.to_be_bytes()); // substFormat
    sub_table.extend_from_slice(&cov_off.to_be_bytes());
    sub_table.extend_from_slice(&1u16.to_be_bytes()); // ligatureSetCount
    sub_table.extend_from_slice(&lig_set_off.to_be_bytes());
    sub_table.extend_from_slice(&coverage);
    sub_table.extend_from_slice(&lig_set);

    let mut lookup = Vec::new();
    lookup.extend_from_slice(&4u16.to_be_bytes()); // lookupType 4
    lookup.extend_from_slice(&0u16.to_be_bytes()); // lookupFlag
    lookup.extend_from_slice(&1u16.to_be_bytes()); // subTableCount
    lookup.extend_from_slice(&8u16.to_be_bytes()); // subTableOffsets[0]
    lookup.extend_from_slice(&sub_table);

    let mut lookup_list = Vec::new();
    lookup_list.extend_from_slice(&1u16.to_be_bytes()); // lookupCount
    lookup_list.extend_from_slice(&4u16.to_be_bytes()); // lookupOffsets[0]
    lookup_list.extend_from_slice(&lookup);

    assemble_layout_table(b"liga", &lookup_list)
}

// ---------------------------------------------------------------------------
// GPOS: one LookupType-5 (MarkLigPos) lookup.
// ---------------------------------------------------------------------------

/// Build a GPOS table publishing a `mark` feature under script `DFLT`
/// whose single LookupType-5 sub-table attaches `mark_gid` (mark class
/// 0, anchor at origin) to `lig_gid`, with one class-0 anchor per
/// component in `component_anchors`.
fn build_gpos_mark_to_ligature(
    mark_gid: u16,
    lig_gid: u16,
    component_anchors: &[(i16, i16)],
) -> Vec<u8> {
    let mark_class_count: u16 = 1;

    // --- MarkArray: one MarkRecord (class 0) + the mark's Anchor. ---
    // MarkArray layout: markCount + markRecords[(markClass, anchorOff)]
    //   followed by Anchor tables.
    let mark_array_header = 2u16 + 4; // markCount + 1 MarkRecord (4 bytes)
    let mark_anchor_off = mark_array_header; // anchor right after header
    let mut mark_array = Vec::new();
    mark_array.extend_from_slice(&1u16.to_be_bytes()); // markCount
    mark_array.extend_from_slice(&0u16.to_be_bytes()); // markClass = 0
    mark_array.extend_from_slice(&mark_anchor_off.to_be_bytes()); // anchorOffset
    mark_array.extend_from_slice(&1u16.to_be_bytes()); // anchorFormat 1
    mark_array.extend_from_slice(&0i16.to_be_bytes()); // x = 0
    mark_array.extend_from_slice(&0i16.to_be_bytes()); // y = 0

    // --- LigatureArray: one LigatureAttach with `n` ComponentRecords,
    // each carrying markClassCount(=1) Offset16 to a class-0 Anchor. ---
    let n = component_anchors.len() as u16;
    // LigatureAttach: componentCount + componentRecords[n]
    //   each ComponentRecord = ligatureAnchorOffsets[markClassCount]
    let lig_attach_header = 2 + 2 * mark_class_count * n; // componentCount + offsets
    let mut lig_attach = Vec::new();
    lig_attach.extend_from_slice(&n.to_be_bytes()); // componentCount
                                                    // Anchors are laid out after all ComponentRecords, one per component.
    let anchors_base = lig_attach_header;
    let anchor_len = 6u16; // Anchor Format 1: fmt + x + y
    for k in 0..n {
        let off = anchors_base + k * anchor_len;
        lig_attach.extend_from_slice(&off.to_be_bytes()); // class-0 anchor offset
    }
    for &(x, y) in component_anchors {
        lig_attach.extend_from_slice(&1u16.to_be_bytes()); // anchorFormat 1
        lig_attach.extend_from_slice(&x.to_be_bytes());
        lig_attach.extend_from_slice(&y.to_be_bytes());
    }

    // LigatureArray: ligatureCount + ligatureAttachOffsets[1] + attach.
    let lig_array_header = 4u16; // ligatureCount + 1 offset
    let mut lig_array = Vec::new();
    lig_array.extend_from_slice(&1u16.to_be_bytes()); // ligatureCount
    lig_array.extend_from_slice(&lig_array_header.to_be_bytes()); // offset[0]
    lig_array.extend_from_slice(&lig_attach);

    // --- MarkLigPosFormat1 subtable. ---
    // format + markCoverageOff + ligCoverageOff + markClassCount +
    //   markArrayOff + ligArrayOff, then the four sub-blocks.
    let sub_header = 12u16; // 6 * 2 bytes
    let mark_cov_off = sub_header;
    let mut mark_cov = Vec::new();
    mark_cov.extend_from_slice(&1u16.to_be_bytes()); // coverageFormat 1
    mark_cov.extend_from_slice(&1u16.to_be_bytes()); // glyphCount
    mark_cov.extend_from_slice(&mark_gid.to_be_bytes());

    let lig_cov_off = mark_cov_off + mark_cov.len() as u16;
    let mut lig_cov = Vec::new();
    lig_cov.extend_from_slice(&1u16.to_be_bytes()); // coverageFormat 1
    lig_cov.extend_from_slice(&1u16.to_be_bytes()); // glyphCount
    lig_cov.extend_from_slice(&lig_gid.to_be_bytes());

    let mark_array_off = lig_cov_off + lig_cov.len() as u16;
    let lig_array_off = mark_array_off + mark_array.len() as u16;

    let mut sub = Vec::new();
    sub.extend_from_slice(&1u16.to_be_bytes()); // format = 1
    sub.extend_from_slice(&mark_cov_off.to_be_bytes());
    sub.extend_from_slice(&lig_cov_off.to_be_bytes());
    sub.extend_from_slice(&mark_class_count.to_be_bytes());
    sub.extend_from_slice(&mark_array_off.to_be_bytes());
    sub.extend_from_slice(&lig_array_off.to_be_bytes());
    sub.extend_from_slice(&mark_cov);
    sub.extend_from_slice(&lig_cov);
    sub.extend_from_slice(&mark_array);
    sub.extend_from_slice(&lig_array);

    let mut lookup = Vec::new();
    lookup.extend_from_slice(&5u16.to_be_bytes()); // lookupType 5
    lookup.extend_from_slice(&0u16.to_be_bytes()); // lookupFlag
    lookup.extend_from_slice(&1u16.to_be_bytes()); // subTableCount
    lookup.extend_from_slice(&8u16.to_be_bytes()); // subTableOffsets[0]
    lookup.extend_from_slice(&sub);

    let mut lookup_list = Vec::new();
    lookup_list.extend_from_slice(&1u16.to_be_bytes()); // lookupCount
    lookup_list.extend_from_slice(&4u16.to_be_bytes()); // lookupOffsets[0]
    lookup_list.extend_from_slice(&lookup);

    assemble_layout_table(b"mark", &lookup_list)
}

/// Shared GSUB/GPOS Script/Feature/Lookup wrapper: build the
/// ScriptList(DFLT) + FeatureList(`tag`) + header around a prebuilt
/// LookupList. The single feature references lookup index 0.
fn assemble_layout_table(feature_tag: &[u8; 4], lookup_list: &[u8]) -> Vec<u8> {
    let mut feature_record = Vec::new();
    feature_record.extend_from_slice(&0u16.to_be_bytes()); // featureParamsOffset
    feature_record.extend_from_slice(&1u16.to_be_bytes()); // lookupIndexCount
    feature_record.extend_from_slice(&0u16.to_be_bytes()); // lookupListIndices[0]

    let mut feature_list = Vec::new();
    feature_list.extend_from_slice(&1u16.to_be_bytes()); // featureCount
    feature_list.extend_from_slice(feature_tag);
    feature_list.extend_from_slice(&8u16.to_be_bytes()); // featureOffset (2 + 6)
    feature_list.extend_from_slice(&feature_record);

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
    script_list.extend_from_slice(b"DFLT");
    script_list.extend_from_slice(&8u16.to_be_bytes()); // scriptOffset (2 + 6)
    script_list.extend_from_slice(&script);

    let header_len: u16 = 10;
    let script_list_off = header_len;
    let feature_list_off = script_list_off + script_list.len() as u16;
    let lookup_list_off = feature_list_off + feature_list.len() as u16;

    let mut table = Vec::new();
    table.extend_from_slice(&1u16.to_be_bytes()); // majorVersion
    table.extend_from_slice(&0u16.to_be_bytes()); // minorVersion
    table.extend_from_slice(&script_list_off.to_be_bytes());
    table.extend_from_slice(&feature_list_off.to_be_bytes());
    table.extend_from_slice(&lookup_list_off.to_be_bytes());
    table.extend_from_slice(&script_list);
    table.extend_from_slice(&feature_list);
    table.extend_from_slice(lookup_list);
    pad_to_4(&mut table);
    table
}

// ---------------------------------------------------------------------------
// GDEF: GlyphClassDef Format 2 flagging `mark_gid` as class 3 (Mark).
// ---------------------------------------------------------------------------

fn build_gdef(mark_gid: u16) -> Vec<u8> {
    // ClassDefFormat2: classFormat + classRangeCount +
    //   ClassRangeRecord[(startGID, endGID, class)].
    let mut class_def = Vec::new();
    class_def.extend_from_slice(&2u16.to_be_bytes()); // classFormat 2
    class_def.extend_from_slice(&1u16.to_be_bytes()); // classRangeCount
    class_def.extend_from_slice(&mark_gid.to_be_bytes()); // startGlyphID
    class_def.extend_from_slice(&mark_gid.to_be_bytes()); // endGlyphID
    class_def.extend_from_slice(&3u16.to_be_bytes()); // class 3 = Mark

    // GDEF header v1.0: 12 bytes of offsets; only glyphClassDef set.
    let header_len: u16 = 12;
    let mut gdef = Vec::new();
    gdef.extend_from_slice(&0x00010000u32.to_be_bytes()); // version 1.0
    gdef.extend_from_slice(&header_len.to_be_bytes()); // glyphClassDefOffset
    gdef.extend_from_slice(&0u16.to_be_bytes()); // attachListOffset
    gdef.extend_from_slice(&0u16.to_be_bytes()); // ligCaretListOffset
    gdef.extend_from_slice(&0u16.to_be_bytes()); // markAttachClassDefOffset
    gdef.extend_from_slice(&class_def);
    pad_to_4(&mut gdef);
    gdef
}

// ---------------------------------------------------------------------------
// Whole-font assembly.
// ---------------------------------------------------------------------------

/// Build the synthetic TTF: 5 glyphs (`.notdef`, 'f', 'i', fi-lig,
/// acute), cmap mapping f/i/acute → gids 1/2/4, plus GDEF + GSUB
/// (ligature) + GPOS (mark-to-ligature) when requested.
fn build_synthetic_ttf(
    with_gpos: Option<&[(i16, i16)]>,
    with_gdef: bool,
    with_gsub: bool,
) -> Vec<u8> {
    const NUM_GLYPHS: u16 = 5;
    const FIRST: u16 = 1; // 'f'
    const SECOND: u16 = 2; // 'i'
    const LIG: u16 = 3; // fi-ligature
    const MARK: u16 = 4; // combining acute

    let glyf: Vec<u8> = Vec::new();
    let mut loca = Vec::new();
    for _ in 0..=NUM_GLYPHS {
        loca.extend_from_slice(&0u16.to_be_bytes());
    }
    pad_to_4(&mut loca);

    let mut maxp = Vec::new();
    maxp.extend_from_slice(&0x00005000u32.to_be_bytes());
    maxp.extend_from_slice(&NUM_GLYPHS.to_be_bytes());
    pad_to_4(&mut maxp);

    let mut head = Vec::new();
    head.extend_from_slice(&0x00010000u32.to_be_bytes());
    head.extend_from_slice(&0x00010000u32.to_be_bytes());
    head.extend_from_slice(&0u32.to_be_bytes());
    head.extend_from_slice(&0x5F0F3CF5u32.to_be_bytes());
    head.extend_from_slice(&0u16.to_be_bytes());
    head.extend_from_slice(&1000u16.to_be_bytes()); // unitsPerEm
    head.extend_from_slice(&0i64.to_be_bytes());
    head.extend_from_slice(&0i64.to_be_bytes());
    head.extend_from_slice(&0i16.to_be_bytes());
    head.extend_from_slice(&0i16.to_be_bytes());
    head.extend_from_slice(&0i16.to_be_bytes());
    head.extend_from_slice(&0i16.to_be_bytes());
    head.extend_from_slice(&0u16.to_be_bytes());
    head.extend_from_slice(&8u16.to_be_bytes());
    head.extend_from_slice(&2i16.to_be_bytes());
    head.extend_from_slice(&0i16.to_be_bytes()); // indexToLocFormat (short)
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
    hhea.extend_from_slice(&NUM_GLYPHS.to_be_bytes()); // numberOfHMetrics
    pad_to_4(&mut hhea);

    let mut hmtx = Vec::new();
    for _ in 0..NUM_GLYPHS {
        hmtx.extend_from_slice(&500u16.to_be_bytes());
        hmtx.extend_from_slice(&0i16.to_be_bytes());
    }
    pad_to_4(&mut hmtx);

    let mut name = Vec::new();
    name.extend_from_slice(&0u16.to_be_bytes());
    name.extend_from_slice(&0u16.to_be_bytes());
    name.extend_from_slice(&6u16.to_be_bytes());
    pad_to_4(&mut name);

    // cmap format 12: f → 1, i → 2, acute → 4 (the ligature gid 3 has
    // no cmap entry — it only appears via GSUB).
    let mut groups: Vec<(u32, u32, u32)> = vec![
        ('f' as u32, 'f' as u32, FIRST as u32),
        ('i' as u32, 'i' as u32, SECOND as u32),
        (0x0301, 0x0301, MARK as u32),
    ];
    groups.sort_by_key(|g| g.0);
    let mut sub = Vec::new();
    sub.extend_from_slice(&12u16.to_be_bytes());
    sub.extend_from_slice(&0u16.to_be_bytes());
    let total_len = 16u32 + 12u32 * groups.len() as u32;
    sub.extend_from_slice(&total_len.to_be_bytes());
    sub.extend_from_slice(&0u32.to_be_bytes());
    sub.extend_from_slice(&(groups.len() as u32).to_be_bytes());
    for (start, end, gid) in &groups {
        sub.extend_from_slice(&start.to_be_bytes());
        sub.extend_from_slice(&end.to_be_bytes());
        sub.extend_from_slice(&gid.to_be_bytes());
    }
    let mut cmap = Vec::new();
    cmap.extend_from_slice(&0u16.to_be_bytes());
    cmap.extend_from_slice(&1u16.to_be_bytes());
    cmap.extend_from_slice(&3u16.to_be_bytes());
    cmap.extend_from_slice(&10u16.to_be_bytes());
    cmap.extend_from_slice(&12u32.to_be_bytes());
    cmap.extend_from_slice(&sub);
    pad_to_4(&mut cmap);

    // Tables in ascending tag order.
    let mut tables_meta: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
    if with_gdef {
        tables_meta.push((b"GDEF", build_gdef(MARK)));
    }
    if let Some(comp_anchors) = with_gpos {
        tables_meta.push((
            b"GPOS",
            build_gpos_mark_to_ligature(MARK, LIG, comp_anchors),
        ));
    }
    if with_gsub {
        tables_meta.push((b"GSUB", build_gsub_ligature(FIRST, SECOND, LIG)));
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

// component-0 anchor (120, 700); component-1 anchor (380, 720).
const COMP0: (i16, i16) = (120, 700);
const COMP1: (i16, i16) = (380, 720);

fn shape(face: &Face, text: &str, size_px: f32) -> Vec<PositionedGlyph> {
    Shaper::shape(face, text, size_px).expect("shape")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn mark_attaches_to_last_ligature_component() {
    let bytes = build_synthetic_ttf(Some(&[COMP0, COMP1]), true, true);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");

    // "fi" + combining acute. GSUB collapses f+i → ligature gid 3, then
    // the trailing acute (gid 4) attaches to the LAST component (1).
    let glyphs = shape(&face, "fi\u{0301}", 1000.0);
    assert_eq!(glyphs.len(), 2, "ligature + mark");
    assert_eq!(glyphs[0].glyph_id, 3, "fi-ligature formed");
    assert_eq!(glyphs[1].glyph_id, 4, "combining acute follows");

    // The mark attaches to the LAST component (1), anchor (380, 720).
    // The established mark-attachment math expresses the offset as a
    // delta from the natural pen position: the mark's pen sits one
    // advance (500) past the ligature, so
    //   x_offset = component_anchor.x − mark_anchor.x − intervening_adv
    //            = 380 − 0 − 500 = −120
    // and (TT Y-up → raster Y-down)
    //   y_offset = −(component_anchor.y − mark_anchor.y) = −720.
    assert_eq!(glyphs[1].x_offset, COMP1.0 as f32 - 500.0);
    assert_eq!(glyphs[1].y_offset, -(COMP1.1 as f32));
    // The mark's advance is zeroed so following glyphs are unaffected.
    assert_eq!(glyphs[1].x_advance, 0.0);
}

#[test]
fn mark_to_ligature_scales_with_size() {
    let bytes = build_synthetic_ttf(Some(&[COMP0, COMP1]), true, true);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");

    // scale 0.5 at 500 px on a upem-1000 font: anchors AND the
    // intervening advance halve. x_offset = (380 − 500) × 0.5 = −60,
    // y_offset = −720 × 0.5 = −360.
    let glyphs = shape(&face, "fi\u{0301}", 500.0);
    assert_eq!(glyphs.len(), 2);
    assert_eq!(glyphs[1].x_offset, (COMP1.0 as f32 - 500.0) * 0.5);
    assert_eq!(glyphs[1].y_offset, -(COMP1.1 as f32) * 0.5);
}

#[test]
fn second_component_null_anchor_falls_back_to_first() {
    // Component 1 publishes a NULL class-0 anchor (offset 0); the mark
    // must walk to component 0 and land there. We synthesise the NULL
    // by building a one-component LigatureAttach (componentCount = 1)
    // while the ligature still collapses 2 source glyphs — so the
    // shaper asks for component 1 (out of range → None) then falls
    // back to component 0.
    let bytes = build_synthetic_ttf(Some(&[COMP0]), true, true);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");

    let glyphs = shape(&face, "fi\u{0301}", 1000.0);
    assert_eq!(glyphs.len(), 2);
    assert_eq!(glyphs[0].glyph_id, 3);
    // Falls back to component 0's anchor (120, 700):
    //   x_offset = 120 − 0 − 500 = −380, y_offset = −700.
    assert_eq!(glyphs[1].x_offset, COMP0.0 as f32 - 500.0);
    assert_eq!(glyphs[1].y_offset, -(COMP0.1 as f32));
}

#[test]
fn no_ligature_keeps_mark_unattached_by_type5() {
    // Without GSUB the run does not form a ligature, so there is no
    // multi-component base; the type-5 path never fires. With no
    // mark-to-base anchor for the plain 'i' either, the mark stays at
    // its cmap position (no offset).
    let bytes = build_synthetic_ttf(Some(&[COMP0, COMP1]), true, false);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");

    let glyphs = shape(&face, "fi\u{0301}", 1000.0);
    assert_eq!(glyphs.len(), 3, "f, i, acute — no ligature");
    assert_eq!(
        glyphs.iter().map(|g| g.glyph_id).collect::<Vec<_>>(),
        [1, 2, 4]
    );
    // The mark is GDEF-classified but its base (gid 2) is not in the
    // type-5 ligature coverage and there is no mark-to-base anchor, so
    // no positioning delta is applied.
    assert_eq!(glyphs[2].x_offset, 0.0);
    assert_eq!(glyphs[2].y_offset, 0.0);
}

#[test]
fn font_without_gpos_is_a_no_op() {
    let bytes = build_synthetic_ttf(None, true, true);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    let glyphs = shape(&face, "fi\u{0301}", 1000.0);
    // Ligature still forms via GSUB; the mark gets no GPOS adjustment.
    assert_eq!(glyphs[0].glyph_id, 3);
    assert_eq!(glyphs[1].glyph_id, 4);
    assert_eq!(glyphs[1].x_offset, 0.0);
    assert_eq!(glyphs[1].y_offset, 0.0);
}

#[test]
fn ligature_forms_independently_of_mark() {
    // Bare "fi" (no mark) still collapses to the ligature glyph — the
    // round-304 component-count tracking must not disturb plain
    // ligature substitution.
    let bytes = build_synthetic_ttf(Some(&[COMP0, COMP1]), true, true);
    let face = Face::from_ttf_bytes(bytes).expect("parse synthetic TTF");
    let glyphs = shape(&face, "fi", 1000.0);
    assert_eq!(glyphs.len(), 1);
    assert_eq!(glyphs[0].glyph_id, 3);
    assert_eq!(glyphs[0].x_advance, 500.0);
}
