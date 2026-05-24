//! Round 125 — GSUB LookupType 2 (Multiple Substitution, Format 1)
//! wired into [`Face::shape_text`] alongside the round-89 LookupType 1
//! single-substitution path.
//!
//! LookupType 2 takes one input glyph and emits a sequence of N output
//! glyphs (OpenType §6.2.2). The classic use case is `ccmp` "split
//! a precomposed glyph into base + combining mark" so a subsequent
//! GPOS mark-attachment pass can position the mark independently. The
//! brief for round 125 wires Format 1 only; Format 1 is the only
//! format the spec defines for LookupType 2.
//!
//! ## Why a synthetic fixture
//!
//! Both vendored fixtures (DejaVu Sans + Inter Variable) publish zero
//! LookupType-2 lookups across their entire GSUB tables (probed at
//! 2026-05-25: DejaVu has types `{1: 23, 3: 1, 4: 12, 6: 4}`; Inter
//! has `{1: 46, 3: 1, 4: 14, 5: 2, 6: 7}`). To exercise the new code
//! path with a fixture, we build a **minimal synthetic TTF in-test**:
//! 4 glyphs (`.notdef` + 'a' + 'b' + 'c'), a Format-4 cmap mapping
//! 'a'/'b'/'c' to GIDs 1/2/3, and a GSUB table that publishes one
//! feature (`ccmp` under script `DFLT`) wrapping a single
//! MultipleSubstFormat1 lookup that splits gid 1 ('a') into `[2, 3]`
//! ('b' followed by 'c').
//!
//! The synthetic-font builder lives entirely in this test file. No
//! external library was consulted; every byte layout follows the
//! Microsoft Typography OpenType spec ("OpenType Specification 1.9.1"
//! chapter 5 + chapter 6 §6.2.2 + the "GSUB Header" / "Coverage Table
//! Formats" common-table sections, transcribed from the published
//! tables, no HarfBuzz / FreeType / Pango / Skia source).
//!
//! ## What this exercises
//!
//! - One-to-many expansion: shaping "a" with the `ccmp` feature
//!   returns 2 glyphs instead of 1.
//! - Length contract: the returned `Vec` carries the post-substitution
//!   length, not the cmap'd input length.
//! - Coverage gating: shaping "b" with the same feature is a no-op
//!   (gid 2 isn't in the Type-2 lookup's coverage).
//! - Mixed input: shaping "ab" expands the 'a' slot and leaves the
//!   'b' slot intact (output is `[2, 3, 2]`).
//! - Idempotence guard: re-applying the same Type-2 lookup after a
//!   first hit must not re-match its own output — the walker
//!   advances past the inserted sequence.
//! - Empty-features baseline: shaping "a" without features stays at
//!   cmap identity (1 glyph).

use oxideav_scribe::Face;

// ---------------------------------------------------------------------------
// Synthetic-font builder
// ---------------------------------------------------------------------------

fn pad_to_4(buf: &mut Vec<u8>) {
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

/// Sum a byte slice as big-endian u32 words for the sfnt table-record
/// checksum field. Pads to a 4-byte boundary with zeros (per the
/// OpenType spec).
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

/// Build a minimal synthetic TTF with 4 glyphs (.notdef + a + b + c)
/// and a GSUB table that publishes one `ccmp` feature under script
/// `DFLT` with one LookupType-2 (Multiple Substitution, Format 1)
/// lookup mapping the covered `input_gid` to the supplied
/// `sequence`. The classic non-deletion case is
/// `build_synthetic_ttf_with_subst(1, &[2, 3])`; the spec also
/// permits `glyphCount = 0` (deletion) — passing an empty `sequence`
/// produces exactly that.
fn build_synthetic_ttf_with_subst(input_gid: u16, sequence: &[u16]) -> Vec<u8> {
    // ----- glyf: 4 empty glyphs (all .notdef-like, no contours). ---------
    // Each glyph is just the empty record (numberOfContours = 0 absent
    // means "no glyph entry"); we use an empty glyf with a loca that
    // points every glyph to the same zero-length slot.
    let glyf: Vec<u8> = Vec::new();
    pad_to_4(&mut Vec::new()); // no-op, glyf is already aligned

    // ----- loca: 5 u16 offsets (numGlyphs + 1), all zero. ----------------
    // head.indexToLocFormat = 0 → short loca → offset = stored * 2,
    // so all-zeros means every glyph has zero length and starts at 0.
    let mut loca = Vec::new();
    for _ in 0..5 {
        loca.extend_from_slice(&0u16.to_be_bytes());
    }
    pad_to_4(&mut loca);

    // ----- maxp 0.5 (TrueType): version + numGlyphs. ---------------------
    let mut maxp = Vec::new();
    maxp.extend_from_slice(&0x00005000u32.to_be_bytes()); // version 0.5
    maxp.extend_from_slice(&4u16.to_be_bytes()); // numGlyphs = 4
    pad_to_4(&mut maxp);

    // ----- head ----------------------------------------------------------
    let mut head = Vec::new();
    head.extend_from_slice(&0x00010000u32.to_be_bytes()); // version 1.0
    head.extend_from_slice(&0x00010000u32.to_be_bytes()); // fontRevision 1.0
    head.extend_from_slice(&0u32.to_be_bytes()); // checkSumAdjustment — patched below
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
    head.extend_from_slice(&2i16.to_be_bytes()); // fontDirectionHint (deprecated)
    head.extend_from_slice(&0i16.to_be_bytes()); // indexToLocFormat (short)
    head.extend_from_slice(&0i16.to_be_bytes()); // glyphDataFormat
    pad_to_4(&mut head);

    // ----- hhea ----------------------------------------------------------
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

    // ----- hmtx: 4 longHorMetric records (advance + lsb each). -----------
    let mut hmtx = Vec::new();
    for _ in 0..4 {
        hmtx.extend_from_slice(&500u16.to_be_bytes()); // advanceWidth
        hmtx.extend_from_slice(&0i16.to_be_bytes()); // lsb
    }
    pad_to_4(&mut hmtx);

    // ----- name: zero records. -------------------------------------------
    let mut name = Vec::new();
    name.extend_from_slice(&0u16.to_be_bytes()); // format
    name.extend_from_slice(&0u16.to_be_bytes()); // count
    name.extend_from_slice(&6u16.to_be_bytes()); // stringOffset (points past header)
    pad_to_4(&mut name);

    // ----- cmap: one subtable, format 4, mapping 'a'/'b'/'c' → 1/2/3. ----
    // Format 4 layout: segCountX2, searchRange, entrySelector,
    // rangeShift, endCode[segCount], reservedPad, startCode[segCount],
    // idDelta[segCount], idRangeOffset[segCount], glyphIdArray[].
    //
    // We use one segment covering 0x61..0x63 ('a'..'c') with idDelta
    // chosen so 'a' (0x61) → gid 1: delta = 1 - 0x61 = -0x60 (mod
    // 0x10000) = 0xFFA0 as u16.
    //
    // The spec also requires a terminating segment ending at 0xFFFF
    // (mapping to gid 0).
    let segcount: u16 = 2;
    let segcountx2 = segcount * 2;
    let search_range = 2u16; // 2 * 2^floor(log2(segcount)) = 2*1 = 2
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
    // endCode[2]: 0x63, 0xFFFF
    sub.extend_from_slice(&0x0063u16.to_be_bytes());
    sub.extend_from_slice(&0xFFFFu16.to_be_bytes());
    sub.extend_from_slice(&0u16.to_be_bytes()); // reservedPad
                                                // startCode[2]: 0x61, 0xFFFF
    sub.extend_from_slice(&0x0061u16.to_be_bytes());
    sub.extend_from_slice(&0xFFFFu16.to_be_bytes());
    // idDelta[2]: -0x60 (= 0xFFA0), 1 (terminator → gid 0)
    sub.extend_from_slice(&0xFFA0u16.to_be_bytes());
    sub.extend_from_slice(&1u16.to_be_bytes());
    // idRangeOffset[2]: 0, 0 (use idDelta directly)
    sub.extend_from_slice(&0u16.to_be_bytes());
    sub.extend_from_slice(&0u16.to_be_bytes());
    // Patch the subtable length.
    let sub_len = sub.len() as u16;
    sub[length_offset..length_offset + 2].copy_from_slice(&sub_len.to_be_bytes());

    // cmap header: version + numTables + EncodingRecord (platform,
    // encoding, offset to subtable).
    let mut cmap = Vec::new();
    cmap.extend_from_slice(&0u16.to_be_bytes()); // version
    cmap.extend_from_slice(&1u16.to_be_bytes()); // numTables
    cmap.extend_from_slice(&3u16.to_be_bytes()); // platformID (Microsoft)
    cmap.extend_from_slice(&1u16.to_be_bytes()); // encodingID (Unicode BMP)
    let subtable_offset = (4 + 8) as u32; // header 4 + one encoding record 8
    cmap.extend_from_slice(&subtable_offset.to_be_bytes());
    cmap.extend_from_slice(&sub);
    pad_to_4(&mut cmap);

    // ----- GSUB ----------------------------------------------------------
    // Lookup: LookupType 2, one sub-table = MultipleSubstFormat1 covering
    // gid 1 with sequence [2, 3].
    //
    // Sub-table layout (§6.2.2):
    //   uint16 substFormat = 1
    //   Offset16 coverageOffset
    //   uint16 sequenceCount
    //   Offset16 sequenceOffsets[sequenceCount]
    //   Coverage table
    //   Sequence record(s): uint16 glyphCount; uint16 substituteGlyphIDs[glyphCount]
    //
    // Coverage Format 1: uint16 fmt = 1; uint16 glyphCount; uint16 glyphArray[]
    let mut sub_table = Vec::new();
    // MultipleSubstFormat1 header is 4 u16 fields (substFormat,
    // coverageOffset, sequenceCount, sequenceOffsets[0]) = 8 bytes;
    // coverage follows immediately at offset 8.
    let cov_off: u16 = 8;
    // Coverage Format 1 covering 1 glyph is 6 bytes (fmt + count + 1
    // glyph id), so the Sequence record starts at offset 8 + 6 = 14.
    let seq_off: u16 = cov_off + 6;
    sub_table.extend_from_slice(&1u16.to_be_bytes()); // substFormat
    sub_table.extend_from_slice(&cov_off.to_be_bytes());
    sub_table.extend_from_slice(&1u16.to_be_bytes()); // sequenceCount
    sub_table.extend_from_slice(&seq_off.to_be_bytes());
    // Coverage Format 1 covering `input_gid`.
    sub_table.extend_from_slice(&1u16.to_be_bytes()); // coverage format
    sub_table.extend_from_slice(&1u16.to_be_bytes()); // glyph count
    sub_table.extend_from_slice(&input_gid.to_be_bytes());
    // Sequence: glyphCount, substituteGlyphIDs[glyphCount]. An empty
    // sequence (`glyphCount = 0`) is the legal deletion form.
    sub_table.extend_from_slice(&(sequence.len() as u16).to_be_bytes());
    for &g in sequence {
        sub_table.extend_from_slice(&g.to_be_bytes());
    }

    // Lookup record: lookupType + lookupFlag + subTableCount + subTableOffsets[1] + subtable
    let mut lookup = Vec::new();
    lookup.extend_from_slice(&2u16.to_be_bytes()); // lookupType = 2 (Multiple)
    lookup.extend_from_slice(&0u16.to_be_bytes()); // lookupFlag
    lookup.extend_from_slice(&1u16.to_be_bytes()); // subTableCount
    let sub_table_offset: u16 = 8; // header 2+2+2+2 = 8
    lookup.extend_from_slice(&sub_table_offset.to_be_bytes());
    lookup.extend_from_slice(&sub_table);

    // Lookup list: lookupCount + lookupOffsets[1] + lookup
    let mut lookup_list = Vec::new();
    lookup_list.extend_from_slice(&1u16.to_be_bytes()); // lookupCount
    let lookup_offset: u16 = 4; // 2 (count) + 2 (one offset) = 4
    lookup_list.extend_from_slice(&lookup_offset.to_be_bytes());
    lookup_list.extend_from_slice(&lookup);

    // Feature: ccmp with one lookup (index 0).
    let mut feature_record = Vec::new();
    feature_record.extend_from_slice(&0u16.to_be_bytes()); // featureParamsOffset
    feature_record.extend_from_slice(&1u16.to_be_bytes()); // lookupIndexCount
    feature_record.extend_from_slice(&0u16.to_be_bytes()); // lookupListIndices[0]

    // Feature list: featureCount + FeatureRecord[1] (tag + offset) + Feature
    let mut feature_list = Vec::new();
    feature_list.extend_from_slice(&1u16.to_be_bytes()); // featureCount
    feature_list.extend_from_slice(b"ccmp"); // tag
    let feature_offset: u16 = 2 + 6; // header 2 + one feature record 6 = 8
    feature_list.extend_from_slice(&feature_offset.to_be_bytes());
    feature_list.extend_from_slice(&feature_record);

    // LangSys: required = 0xFFFF, featureIndexCount = 1, featureIndices = [0]
    let mut langsys = Vec::new();
    langsys.extend_from_slice(&0u16.to_be_bytes()); // lookupOrderOffset (reserved, 0)
    langsys.extend_from_slice(&0xFFFFu16.to_be_bytes()); // requiredFeatureIndex
    langsys.extend_from_slice(&1u16.to_be_bytes()); // featureIndexCount
    langsys.extend_from_slice(&0u16.to_be_bytes()); // featureIndices[0]

    // Script: defaultLangSysOffset + langSysCount + LangSysRecord[] + DefaultLangSys
    let mut script = Vec::new();
    let default_langsys_offset: u16 = 4; // header 2+2 = 4
    script.extend_from_slice(&default_langsys_offset.to_be_bytes());
    script.extend_from_slice(&0u16.to_be_bytes()); // langSysCount
    script.extend_from_slice(&langsys);

    // Script list: scriptCount + ScriptRecord[1] (tag + offset) + Script
    let mut script_list = Vec::new();
    script_list.extend_from_slice(&1u16.to_be_bytes()); // scriptCount
    script_list.extend_from_slice(b"DFLT"); // scriptTag
    let script_offset: u16 = 2 + 6; // header 2 + one script record 6 = 8
    script_list.extend_from_slice(&script_offset.to_be_bytes());
    script_list.extend_from_slice(&script);

    // GSUB header (version 1.0): scriptListOffset + featureListOffset + lookupListOffset
    let header_len: u16 = 10;
    let script_list_off: u16 = header_len;
    let feature_list_off: u16 = script_list_off + script_list.len() as u16;
    let lookup_list_off: u16 = feature_list_off + feature_list.len() as u16;

    let mut gsub = Vec::new();
    gsub.extend_from_slice(&1u16.to_be_bytes()); // major
    gsub.extend_from_slice(&0u16.to_be_bytes()); // minor
    gsub.extend_from_slice(&script_list_off.to_be_bytes());
    gsub.extend_from_slice(&feature_list_off.to_be_bytes());
    gsub.extend_from_slice(&lookup_list_off.to_be_bytes());
    gsub.extend_from_slice(&script_list);
    gsub.extend_from_slice(&feature_list);
    gsub.extend_from_slice(&lookup_list);
    pad_to_4(&mut gsub);

    // ----- Build the sfnt table directory + body. ------------------------
    // sfnt fixed header: 4 bytes (scaler) + 4 u16 (numTables,
    // searchRange, entrySelector, rangeShift) = 12 bytes. Each table
    // record is 16 bytes. We have 9 tables: GSUB, OS/2-less; cmap,
    // glyf, head, hhea, hmtx, loca, maxp, name, GSUB = 9.
    let tables_meta: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"GSUB", gsub),
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"name", name),
    ];
    // Tables are stored alphabetically by tag (per the spec); the
    // table-record array is *also* sorted alphabetically. The list
    // above is already sorted: GSUB < cmap is true because uppercase
    // 'G' (0x47) < lowercase 'c' (0x63).
    let num_tables = tables_meta.len() as u16;

    // searchRange = 16 * 2^floor(log2(numTables))
    let log2 = (num_tables as f64).log2().floor() as u32;
    let search_range = 16u16 * (1u16 << log2);
    let entry_selector = log2 as u16;
    let range_shift = num_tables * 16 - search_range;

    let header_size = 12usize;
    let record_size = 16usize;
    let body_start = header_size + record_size * tables_meta.len();

    // Plan offsets/lengths up front.
    let mut records: Vec<(&[u8; 4], u32, u32, Vec<u8>)> = Vec::new();
    let mut cursor = body_start as u32;
    for (tag, data) in tables_meta {
        let checksum = table_checksum(&data);
        let length = data.len() as u32;
        records.push((tag, checksum, length, data));
        // 4-byte pad already applied inside each builder.
        cursor += length;
        while cursor % 4 != 0 {
            cursor += 1;
        }
    }
    // Reset cursor for actual write.
    let mut out = Vec::with_capacity(cursor as usize);
    out.extend_from_slice(&0x00010000u32.to_be_bytes()); // scaler ("true" TT)
    out.extend_from_slice(&num_tables.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&range_shift.to_be_bytes());

    // Reserve record area; fill body first then patch offsets.
    out.resize(body_start, 0);
    let mut offsets: Vec<u32> = Vec::with_capacity(records.len());
    for (_tag, _ck, _len, data) in &records {
        let offset = out.len() as u32;
        offsets.push(offset);
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    // Patch records.
    for (i, (tag, checksum, length, _)) in records.iter().enumerate() {
        let rec_pos = header_size + i * record_size;
        out[rec_pos..rec_pos + 4].copy_from_slice(*tag);
        out[rec_pos + 4..rec_pos + 8].copy_from_slice(&checksum.to_be_bytes());
        out[rec_pos + 8..rec_pos + 12].copy_from_slice(&offsets[i].to_be_bytes());
        out[rec_pos + 12..rec_pos + 16].copy_from_slice(&length.to_be_bytes());
    }
    out
}

/// The canonical "split gid 1 into [gid 2, gid 3]" synthetic font
/// used by most of the tests below.
fn synth_face() -> Face {
    let bytes = build_synthetic_ttf_with_subst(1, &[2, 3]);
    Face::from_ttf_bytes(bytes).expect("synthetic TTF parses")
}

/// Variant: the LookupType-2 lookup deletes gid 1 (Sequence record
/// with `glyphCount = 0`, which the spec explicitly permits).
fn synth_face_deletion() -> Face {
    let bytes = build_synthetic_ttf_with_subst(1, &[]);
    Face::from_ttf_bytes(bytes).expect("synthetic TTF parses")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn synthetic_font_cmap_routes_a_b_c() {
    // Sanity check: the synthetic font's cmap really maps 'a'/'b'/'c'
    // to GIDs 1/2/3. Without this the type-2 tests below would be
    // meaningless.
    let face = synth_face();
    face.with_font(|font| {
        assert_eq!(font.glyph_index('a'), Some(1));
        assert_eq!(font.glyph_index('b'), Some(2));
        assert_eq!(font.glyph_index('c'), Some(3));
        // Unmapped char falls through to .notdef.
        assert_eq!(font.glyph_index('z').unwrap_or(0), 0);
    })
    .unwrap();
}

#[test]
fn synthetic_font_publishes_ccmp_under_dflt() {
    // The shaper resolves features under `latn` / `cyrl` / `grek` /
    // `DFLT` in priority order. Our synthetic font only declares
    // `DFLT`, so the script-walk falls through to the last entry.
    let face = synth_face();
    assert!(face.has_gsub_feature(*b"DFLT", *b"ccmp"));
    assert!(
        !face.has_gsub_feature(*b"DFLT", *b"liga"),
        "synthetic font only carries ccmp"
    );
}

#[test]
fn ccmp_splits_a_into_b_c_via_lookup_type_2() {
    // The headline contract: shaping "a" with the `ccmp` feature
    // expands the single-glyph cmap output into the 2-glyph sequence
    // declared by the Type-2 lookup.
    let face = synth_face();
    let cmap_only = face.shape_text("a", &[]);
    let ccmp_on = face.shape_text("a", &[*b"ccmp"]);
    assert_eq!(cmap_only, vec![1u16], "cmap('a') = gid 1");
    assert_eq!(
        ccmp_on,
        vec![2u16, 3u16],
        "ccmp splits gid 1 → [gid 2, gid 3]"
    );
}

#[test]
fn ccmp_is_noop_on_uncovered_glyph() {
    // Coverage gating: the Type-2 lookup covers gid 1 ('a') only.
    // Shaping "b" (cmap → [2]) with `ccmp` must leave the glyph
    // run unchanged because gid 2 isn't in the coverage table.
    let face = synth_face();
    let cmap_only = face.shape_text("b", &[]);
    let ccmp_on = face.shape_text("b", &[*b"ccmp"]);
    assert_eq!(cmap_only, vec![2u16]);
    assert_eq!(ccmp_on, vec![2u16], "gid 2 is outside the type-2 coverage");
}

#[test]
fn ccmp_mixed_input_expands_only_covered_slot() {
    // "ab" cmaps to [1, 2]. The Type-2 lookup fires on the 'a' slot
    // (gid 1 → [2, 3]) and leaves the 'b' slot intact. Result:
    // [2, 3, 2] — length 3 instead of the cmap'd length 2.
    let face = synth_face();
    let out = face.shape_text("ab", &[*b"ccmp"]);
    assert_eq!(out, vec![2u16, 3u16, 2u16]);
}

#[test]
fn ccmp_walker_does_not_re_match_its_own_output() {
    // Idempotence guard. The Type-2 walker advances past the inserted
    // sequence so the same lookup doesn't fire again on its own
    // output. Without that guard, applying `ccmp` twice to "a" would
    // first expand to [2, 3] and then leave it (no recursion) — but
    // calling the feature multiple times must be safe regardless of
    // whether the output happens to be inside coverage. With our
    // coverage {1}, neither gid 2 nor gid 3 ever rematches, so the
    // double-apply collapses to the same output as a single apply.
    let face = synth_face();
    let once = face.shape_text("a", &[*b"ccmp"]);
    let twice = face.shape_text("a", &[*b"ccmp", *b"ccmp"]);
    assert_eq!(once, twice);
    assert_eq!(once, vec![2u16, 3u16]);
}

#[test]
fn ccmp_empty_features_is_cmap_identity_on_a() {
    // The round-89 empty-features baseline must still hold after
    // adding Type-2 dispatch — passing no features = no GSUB pass.
    let face = synth_face();
    assert_eq!(face.shape_text("a", &[]), vec![1u16]);
    assert_eq!(face.shape_text("abc", &[]), vec![1u16, 2u16, 3u16]);
}

#[test]
fn ccmp_empty_text_yields_empty_run() {
    // Empty input is empty output regardless of which lookup types
    // the requested features dispatch.
    let face = synth_face();
    assert_eq!(face.shape_text("", &[*b"ccmp"]).len(), 0);
}

#[test]
fn unknown_feature_skips_type_2_lookup() {
    // Feature-tag resolution misses → no lookup runs → identity.
    let face = synth_face();
    let out = face.shape_text("a", &[*b"zzzz"]);
    assert_eq!(out, vec![1u16]);
}

/// Snapshot of the synthetic font's GSUB shape — the GSUB byte
/// layout in this test is the per-byte clean-room construction of
/// `build_synthetic_ttf_with_subst`, so locking the parsed shape
/// here makes any future drift in either `oxideav-ttf`'s parser or
/// the builder itself surface as a failed assertion rather than a
/// silently broken test.
#[test]
fn synthetic_font_has_one_lookup_type_2() {
    let face = synth_face();
    face.with_font(|font| {
        let ll = font.gsub_lookup_list();
        assert_eq!(ll.len(), 1, "synthetic GSUB declares exactly one lookup");
        let (idx, ty, sub_count) = ll[0];
        assert_eq!(idx, 0);
        assert_eq!(ty, 2, "lookup 0 is LookupType 2 (Multiple Substitution)");
        assert_eq!(
            sub_count, 1,
            "lookup 0 carries one MultipleSubstFormat1 sub-table"
        );
        // Direct accessor: covered gid → declared sequence; uncovered
        // gid → None.
        assert_eq!(font.gsub_apply_lookup_type_2(0, 1), Some(vec![2, 3]));
        assert_eq!(font.gsub_apply_lookup_type_2(0, 2), None);
        assert_eq!(font.gsub_apply_lookup_type_2(0, 3), None);
    })
    .unwrap();
}

#[test]
fn ccmp_lookup_type_2_glyph_count_zero_deletes_input() {
    // The OpenType spec (§6.2.2) explicitly permits a `Sequence`
    // record with `glyphCount = 0`; the dispatcher must surface that
    // as a deletion (the covered slot disappears from the run, no
    // replacement glyph).
    //
    // With our deletion-variant font, shaping "a" must collapse the
    // single-glyph cmap output to an empty run, and "ab" must
    // produce a 1-glyph run (just 'b').
    let face = synth_face_deletion();
    let just_a = face.shape_text("a", &[*b"ccmp"]);
    assert!(
        just_a.is_empty(),
        "glyphCount = 0 deletes the covered slot; got {just_a:?}"
    );
    let ab = face.shape_text("ab", &[*b"ccmp"]);
    assert_eq!(ab, vec![2u16], "the 'b' slot survives the deletion of 'a'");
    // And the cmap-only baseline is untouched (no GSUB pass).
    assert_eq!(face.shape_text("a", &[]), vec![1u16]);
}
