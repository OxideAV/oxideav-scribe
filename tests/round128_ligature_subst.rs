//! Round 128 — GSUB LookupType 4 (Ligature Substitution, Format 1)
//! wired into [`Face::shape_text`] alongside the round-89 LookupType 1
//! (Single Substitution) and round-125 LookupType 2 (Multiple
//! Substitution) paths.
//!
//! LookupType 4 takes a sequence of N input "component" glyphs and
//! emits a single "ligature" glyph (OpenType §6.2.4). The classic
//! use case is `liga` collapsing 'f'+'i' into the fi-ligature glyph
//! so the run gets typographically-correct overlap and kerning. The
//! brief for round 128 wires Format 1 only; Format 1 is the only
//! format the spec defines for LookupType 4.
//!
//! ## Why both real and synthetic fixtures
//!
//! DejaVu Sans publishes a real `liga` LookupType-4 lookup (the
//! standard fi / fl / ffi / ffl English ligatures) under `latn`,
//! which is enough to exercise the headline contract end-to-end —
//! that test lives inline in `src/shaping/feature_subst.rs`. The
//! synthetic-fixture tests here cover the edge cases real fonts
//! don't expose cleanly:
//!
//! - **`componentCount = 1` (single-component ligature)** — legal per
//!   the spec but vanishingly rare in real fonts. Effectively a
//!   single-substitution dressed as a ligature.
//! - **Multiple ligatures in one LigatureSet** — verifying the
//!   longest-match-first rule the spec mandates ("Ligatures whose
//!   first component is the same glyph should be ordered with the
//!   longest ligatures first" — OpenType §6.2.4 Format 1).
//! - **Multiple LigatureSets in one lookup** — verifying that the
//!   coverage table routes to the right LigatureSet per starting
//!   glyph.
//! - **Idempotence** — re-applying the same lookup to its own output
//!   must not loop or re-collapse.
//!
//! ## Clean-room note
//!
//! The synthetic-font builder lives entirely in this test file. No
//! external library was consulted; every byte layout follows the
//! Microsoft Typography OpenType spec ("OpenType Specification
//! 1.9.1" chapter 5 + chapter 6 §6.2.4 + the "GSUB Header" /
//! "Coverage Table Formats" / "Class Definition Table" common-table
//! sections, transcribed from the published tables). No HarfBuzz /
//! FreeType / Pango / Skia source consulted; no WebSearch /
//! WebFetch invoked during this round.

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

/// One declaration of a ligature record inside a LigatureSet.
/// `components` is the trailing-component sequence (positions 2..N);
/// the first component is implied by the LigatureSet's coverage
/// glyph. `lig_glyph` is the replacement GID emitted on match.
#[derive(Clone, Copy)]
struct LigatureRecord<'a> {
    lig_glyph: u16,
    components: &'a [u16],
}

/// One LigatureSet: bound to a "first component" glyph (which goes
/// into the lookup's coverage table) plus the per-set ligature
/// records.
#[derive(Clone, Copy)]
struct LigatureSet<'a> {
    first: u16,
    ligatures: &'a [LigatureRecord<'a>],
}

/// Build a minimal synthetic TTF with `num_glyphs` empty glyphs
/// (every glyph is a zero-contour outline) plus a cmap and a GSUB
/// table carrying one `liga` feature under script `DFLT` with one
/// LookupType-4 (Ligature Substitution, Format 1) lookup populated
/// from `sets`.
///
/// The cmap is parameterised: `chars` is a slice of `(char, gid)`
/// mappings. Glyph 0 stays `.notdef`. Anything not in `chars` maps
/// to `.notdef` as well.
fn build_synthetic_ttf(num_glyphs: u16, chars: &[(char, u16)], sets: &[LigatureSet]) -> Vec<u8> {
    // ----- glyf: zero contours per glyph; loca uses short format and
    // points every glyph to offset 0 (zero-length entry).
    let glyf: Vec<u8> = Vec::new();
    let mut loca = Vec::new();
    for _ in 0..=num_glyphs {
        loca.extend_from_slice(&0u16.to_be_bytes());
    }
    pad_to_4(&mut loca);

    // ----- maxp 0.5: version + numGlyphs.
    let mut maxp = Vec::new();
    maxp.extend_from_slice(&0x00005000u32.to_be_bytes());
    maxp.extend_from_slice(&num_glyphs.to_be_bytes());
    pad_to_4(&mut maxp);

    // ----- head.
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

    // ----- hhea.
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
        hhea.extend_from_slice(&0i16.to_be_bytes());
    }
    hhea.extend_from_slice(&0i16.to_be_bytes()); // metricDataFormat
    hhea.extend_from_slice(&num_glyphs.to_be_bytes()); // numberOfHMetrics
    pad_to_4(&mut hhea);

    // ----- hmtx: one longHorMetric per glyph.
    let mut hmtx = Vec::new();
    for _ in 0..num_glyphs {
        hmtx.extend_from_slice(&500u16.to_be_bytes());
        hmtx.extend_from_slice(&0i16.to_be_bytes());
    }
    pad_to_4(&mut hmtx);

    // ----- name: zero records.
    let mut name = Vec::new();
    name.extend_from_slice(&0u16.to_be_bytes());
    name.extend_from_slice(&0u16.to_be_bytes());
    name.extend_from_slice(&6u16.to_be_bytes());
    pad_to_4(&mut name);

    // ----- cmap: format 12 segmented coverage (full Unicode range).
    // Easier than format 4 for arbitrary non-contiguous mappings.
    let mut groups: Vec<(u32, u32, u32)> = Vec::with_capacity(chars.len());
    for &(ch, gid) in chars {
        let cp = ch as u32;
        groups.push((cp, cp, gid as u32));
    }
    // Sort by startCharCode (format 12 requires ascending).
    groups.sort_by_key(|g| g.0);

    let mut sub = Vec::new();
    sub.extend_from_slice(&12u16.to_be_bytes()); // format
    sub.extend_from_slice(&0u16.to_be_bytes()); // reserved
    let total_len = 16u32 + 12u32 * groups.len() as u32;
    sub.extend_from_slice(&total_len.to_be_bytes()); // length
    sub.extend_from_slice(&0u32.to_be_bytes()); // language
    sub.extend_from_slice(&(groups.len() as u32).to_be_bytes());
    for (start, end, gid) in &groups {
        sub.extend_from_slice(&start.to_be_bytes());
        sub.extend_from_slice(&end.to_be_bytes());
        sub.extend_from_slice(&gid.to_be_bytes());
    }

    let mut cmap = Vec::new();
    cmap.extend_from_slice(&0u16.to_be_bytes()); // version
    cmap.extend_from_slice(&1u16.to_be_bytes()); // numTables
    cmap.extend_from_slice(&3u16.to_be_bytes()); // platformID
    cmap.extend_from_slice(&10u16.to_be_bytes()); // encodingID (Unicode full)
    let subtable_offset: u32 = 12;
    cmap.extend_from_slice(&subtable_offset.to_be_bytes());
    cmap.extend_from_slice(&sub);
    pad_to_4(&mut cmap);

    // ----- GSUB ----------------------------------------------------------
    // Build one LookupType-4 (Ligature Substitution Format 1) lookup
    // with one sub-table whose coverage covers `sets[i].first` for
    // each i and whose LigatureSets carry the spec-formatted ligature
    // records.

    // Build coverage table (Format 1): listed glyphs are
    // `sets[*].first`. The spec requires the glyph array to be in
    // ascending order, with the coverage index implying the
    // matching LigatureSet index.
    let mut sorted_sets: Vec<&LigatureSet> = sets.iter().collect();
    sorted_sets.sort_by_key(|s| s.first);

    // We'll need the index permutation: coverage index i ↦ original
    // index in `sorted_sets` (i.e. the LigatureSet to emit at slot i).
    // Since we sorted by `first`, the natural order *is* the coverage
    // order; the LigatureSet[] array indexes 1:1 with coverage.

    // Build each LigatureSet bytes.
    let mut lig_sets_bytes: Vec<Vec<u8>> = Vec::with_capacity(sorted_sets.len());
    for set in &sorted_sets {
        // LigatureSet header: ligatureCount + Offset16[count]
        let count = set.ligatures.len() as u16;
        let header_len = 2 + 2 * count as usize;
        // Per-ligature body lengths: 4 + 2*(componentCount - 1).
        let mut bodies: Vec<Vec<u8>> = Vec::with_capacity(count as usize);
        for lig in set.ligatures {
            let mut body = Vec::new();
            body.extend_from_slice(&lig.lig_glyph.to_be_bytes());
            // componentCount = 1 + components.len(): the first
            // component is the coverage glyph; `components` is the
            // tail.
            let comp_count = 1u16 + lig.components.len() as u16;
            body.extend_from_slice(&comp_count.to_be_bytes());
            for &c in lig.components {
                body.extend_from_slice(&c.to_be_bytes());
            }
            bodies.push(body);
        }
        // Compute per-ligature offsets relative to the start of the
        // LigatureSet (so it includes the header).
        let mut offsets: Vec<u16> = Vec::with_capacity(count as usize);
        let mut cursor = header_len as u16;
        for body in &bodies {
            offsets.push(cursor);
            cursor += body.len() as u16;
        }
        let mut buf = Vec::new();
        buf.extend_from_slice(&count.to_be_bytes());
        for off in &offsets {
            buf.extend_from_slice(&off.to_be_bytes());
        }
        for body in &bodies {
            buf.extend_from_slice(body);
        }
        lig_sets_bytes.push(buf);
    }

    // LigatureSubstFormat1 sub-table layout:
    //   uint16 substFormat = 1
    //   Offset16 coverageOffset
    //   uint16 ligatureSetCount
    //   Offset16 ligatureSetOffsets[ligatureSetCount]
    //   Coverage table
    //   LigatureSets...
    let lig_set_count = sorted_sets.len() as u16;
    let sub_header_len = 6 + 2 * lig_set_count as usize; // 6 = 2+2+2

    // Coverage Format 1: fmt + count + glyphArray[count].
    let mut coverage = Vec::new();
    coverage.extend_from_slice(&1u16.to_be_bytes()); // fmt
    coverage.extend_from_slice(&lig_set_count.to_be_bytes());
    for s in &sorted_sets {
        coverage.extend_from_slice(&s.first.to_be_bytes());
    }

    let cov_off = sub_header_len as u16;
    let mut lig_set_offsets: Vec<u16> = Vec::with_capacity(lig_set_count as usize);
    let mut cursor = cov_off + coverage.len() as u16;
    for body in &lig_sets_bytes {
        lig_set_offsets.push(cursor);
        cursor += body.len() as u16;
    }

    let mut sub_table = Vec::new();
    sub_table.extend_from_slice(&1u16.to_be_bytes()); // substFormat
    sub_table.extend_from_slice(&cov_off.to_be_bytes());
    sub_table.extend_from_slice(&lig_set_count.to_be_bytes());
    for off in &lig_set_offsets {
        sub_table.extend_from_slice(&off.to_be_bytes());
    }
    sub_table.extend_from_slice(&coverage);
    for body in &lig_sets_bytes {
        sub_table.extend_from_slice(body);
    }

    // Lookup record: lookupType + lookupFlag + subTableCount +
    // subTableOffsets[1] + subtable.
    let mut lookup = Vec::new();
    lookup.extend_from_slice(&4u16.to_be_bytes()); // lookupType = 4 (Ligature)
    lookup.extend_from_slice(&0u16.to_be_bytes()); // lookupFlag
    lookup.extend_from_slice(&1u16.to_be_bytes()); // subTableCount
    let sub_table_offset: u16 = 8; // 2+2+2+2
    lookup.extend_from_slice(&sub_table_offset.to_be_bytes());
    lookup.extend_from_slice(&sub_table);

    // Lookup list: lookupCount + lookupOffsets[1] + lookup.
    let mut lookup_list = Vec::new();
    lookup_list.extend_from_slice(&1u16.to_be_bytes());
    let lookup_offset: u16 = 4;
    lookup_list.extend_from_slice(&lookup_offset.to_be_bytes());
    lookup_list.extend_from_slice(&lookup);

    // Feature: liga with one lookup (index 0).
    let mut feature_record = Vec::new();
    feature_record.extend_from_slice(&0u16.to_be_bytes()); // featureParamsOffset
    feature_record.extend_from_slice(&1u16.to_be_bytes()); // lookupIndexCount
    feature_record.extend_from_slice(&0u16.to_be_bytes()); // lookupListIndices[0]

    let mut feature_list = Vec::new();
    feature_list.extend_from_slice(&1u16.to_be_bytes()); // featureCount
    feature_list.extend_from_slice(b"liga"); // tag
    let feature_offset: u16 = 2 + 6;
    feature_list.extend_from_slice(&feature_offset.to_be_bytes());
    feature_list.extend_from_slice(&feature_record);

    let mut langsys = Vec::new();
    langsys.extend_from_slice(&0u16.to_be_bytes());
    langsys.extend_from_slice(&0xFFFFu16.to_be_bytes());
    langsys.extend_from_slice(&1u16.to_be_bytes());
    langsys.extend_from_slice(&0u16.to_be_bytes());

    let mut script = Vec::new();
    let default_langsys_offset: u16 = 4;
    script.extend_from_slice(&default_langsys_offset.to_be_bytes());
    script.extend_from_slice(&0u16.to_be_bytes());
    script.extend_from_slice(&langsys);

    let mut script_list = Vec::new();
    script_list.extend_from_slice(&1u16.to_be_bytes());
    script_list.extend_from_slice(b"DFLT");
    let script_offset: u16 = 2 + 6;
    script_list.extend_from_slice(&script_offset.to_be_bytes());
    script_list.extend_from_slice(&script);

    let header_len: u16 = 10;
    let script_list_off: u16 = header_len;
    let feature_list_off: u16 = script_list_off + script_list.len() as u16;
    let lookup_list_off: u16 = feature_list_off + feature_list.len() as u16;

    let mut gsub = Vec::new();
    gsub.extend_from_slice(&1u16.to_be_bytes());
    gsub.extend_from_slice(&0u16.to_be_bytes());
    gsub.extend_from_slice(&script_list_off.to_be_bytes());
    gsub.extend_from_slice(&feature_list_off.to_be_bytes());
    gsub.extend_from_slice(&lookup_list_off.to_be_bytes());
    gsub.extend_from_slice(&script_list);
    gsub.extend_from_slice(&feature_list);
    gsub.extend_from_slice(&lookup_list);
    pad_to_4(&mut gsub);

    // ----- sfnt assembly.
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
    let num_tables = tables_meta.len() as u16;
    let log2 = (num_tables as f64).log2().floor() as u32;
    let search_range = 16u16 * (1u16 << log2);
    let entry_selector = log2 as u16;
    let range_shift = num_tables * 16 - search_range;

    let header_size = 12usize;
    let record_size = 16usize;
    let body_start = header_size + record_size * tables_meta.len();

    let mut records: Vec<(&[u8; 4], u32, u32, Vec<u8>)> = Vec::new();
    let mut cursor = body_start as u32;
    for (tag, data) in tables_meta {
        let checksum = table_checksum(&data);
        let length = data.len() as u32;
        records.push((tag, checksum, length, data));
        cursor += length;
        while cursor % 4 != 0 {
            cursor += 1;
        }
    }

    let mut out = Vec::with_capacity(cursor as usize);
    out.extend_from_slice(&0x00010000u32.to_be_bytes()); // scaler
    out.extend_from_slice(&num_tables.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&range_shift.to_be_bytes());

    out.resize(body_start, 0);
    let mut offsets: Vec<u32> = Vec::with_capacity(records.len());
    for (_t, _c, _l, data) in &records {
        let off = out.len() as u32;
        offsets.push(off);
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    for (i, (tag, checksum, length, _)) in records.iter().enumerate() {
        let rec_pos = header_size + i * record_size;
        out[rec_pos..rec_pos + 4].copy_from_slice(*tag);
        out[rec_pos + 4..rec_pos + 8].copy_from_slice(&checksum.to_be_bytes());
        out[rec_pos + 8..rec_pos + 12].copy_from_slice(&offsets[i].to_be_bytes());
        out[rec_pos + 12..rec_pos + 16].copy_from_slice(&length.to_be_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// Headline scenarios
// ---------------------------------------------------------------------------

/// Build a face that ligates gid 1 + gid 2 ("ab") into gid 4.
fn synth_fi_like() -> Face {
    // 5 glyphs: .notdef + 'a'/1 + 'b'/2 + 'c'/3 + ligature/4
    let chars = [('a', 1u16), ('b', 2u16), ('c', 3u16)];
    let ligs = [LigatureRecord {
        lig_glyph: 4,
        components: &[2u16],
    }];
    let sets = [LigatureSet {
        first: 1,
        ligatures: &ligs,
    }];
    let bytes = build_synthetic_ttf(5, &chars, &sets);
    Face::from_ttf_bytes(bytes).expect("synthetic TTF parses")
}

#[test]
fn synthetic_font_cmap_routes_a_b_c() {
    let face = synth_fi_like();
    face.with_font(|font| {
        assert_eq!(font.glyph_index('a'), Some(1));
        assert_eq!(font.glyph_index('b'), Some(2));
        assert_eq!(font.glyph_index('c'), Some(3));
        assert_eq!(font.glyph_index('z').unwrap_or(0), 0);
    })
    .unwrap();
}

#[test]
fn synthetic_font_publishes_liga_under_dflt() {
    let face = synth_fi_like();
    assert!(face.has_gsub_feature(*b"DFLT", *b"liga"));
    assert!(
        !face.has_gsub_feature(*b"DFLT", *b"smcp"),
        "synthetic font only carries liga"
    );
}

#[test]
fn synthetic_font_has_one_lookup_type_4() {
    let face = synth_fi_like();
    face.with_font(|font| {
        let ll = font.gsub_lookup_list();
        assert_eq!(ll.len(), 1);
        assert_eq!(ll[0].1, 4, "the only lookup is LookupType 4 (Ligature)");
    })
    .unwrap();
}

#[test]
fn liga_collapses_two_components_into_ligature() {
    // Headline contract: shape_text("ab", [liga]) returns [4] (the
    // ligature glyph) instead of the cmap [1, 2].
    let face = synth_fi_like();
    let cmap_only = face.shape_text("ab", &[]);
    let liga_on = face.shape_text("ab", &[*b"liga"]);
    assert_eq!(cmap_only, vec![1u16, 2u16]);
    assert_eq!(liga_on, vec![4u16], "ab collapses to the ligature glyph");
}

#[test]
fn liga_is_noop_on_uncovered_prefix() {
    // The lookup's coverage starts at gid 1. Shaping "bc" (cmap →
    // [2, 3]) with `liga` must leave the run unchanged.
    let face = synth_fi_like();
    let cmap_only = face.shape_text("bc", &[]);
    let liga_on = face.shape_text("bc", &[*b"liga"]);
    assert_eq!(cmap_only, vec![2u16, 3u16]);
    assert_eq!(liga_on, vec![2u16, 3u16]);
}

#[test]
fn liga_is_noop_when_tail_doesnt_match() {
    // The ligature wants 'a' then 'b'. Shaping "ac" (cmap → [1, 3])
    // starts with the covered first component but the trailing
    // component doesn't match the lookup's recorded `b` — the
    // lookup returns None and the cursor advances by 1.
    let face = synth_fi_like();
    let cmap_only = face.shape_text("ac", &[]);
    let liga_on = face.shape_text("ac", &[*b"liga"]);
    assert_eq!(cmap_only, vec![1u16, 3u16]);
    assert_eq!(liga_on, vec![1u16, 3u16]);
}

#[test]
fn liga_mixed_input_collapses_only_matching_prefix() {
    // "abab" cmaps to [1, 2, 1, 2]. Both 'ab' prefixes match the
    // lookup → result is [4, 4].
    let face = synth_fi_like();
    let out = face.shape_text("abab", &[*b"liga"]);
    assert_eq!(out, vec![4u16, 4u16]);
}

#[test]
fn liga_partial_then_match() {
    // "cab" cmaps to [3, 1, 2]. 'c' (gid 3) isn't covered → cursor
    // walks past it. At position 1 the 'a' is covered and the tail
    // 'b' matches → splice to [3, 4].
    let face = synth_fi_like();
    let out = face.shape_text("cab", &[*b"liga"]);
    assert_eq!(out, vec![3u16, 4u16]);
}

#[test]
fn liga_does_not_re_match_its_own_output() {
    // Idempotence: applying the same `liga` feature twice must
    // produce the same output. Our ligature glyph (4) is outside
    // the lookup's coverage, so the second application is a no-op.
    let face = synth_fi_like();
    let once = face.shape_text("ab", &[*b"liga"]);
    let twice = face.shape_text("ab", &[*b"liga", *b"liga"]);
    assert_eq!(once, vec![4u16]);
    assert_eq!(twice, vec![4u16]);
}

#[test]
fn liga_empty_text_yields_empty_run() {
    let face = synth_fi_like();
    assert_eq!(face.shape_text("", &[*b"liga"]).len(), 0);
}

#[test]
fn liga_empty_features_is_cmap_identity_on_ab() {
    let face = synth_fi_like();
    let out = face.shape_text("ab", &[]);
    assert_eq!(out, vec![1u16, 2u16]);
}

#[test]
fn unknown_feature_skips_type_4_lookup() {
    let face = synth_fi_like();
    let cmap_only = face.shape_text("ab", &[]);
    let unknown = face.shape_text("ab", &[*b"zzzz"]);
    assert_eq!(cmap_only, unknown);
}

// ---------------------------------------------------------------------------
// Multi-ligature LigatureSet — longest-match-first contract
// ---------------------------------------------------------------------------

/// Build a face where gid 1 has TWO ligature records: the 2-glyph
/// 'a' + 'b' → 4, and the 3-glyph 'a' + 'b' + 'c' → 5. Per the spec
/// the longer one is listed first.
fn synth_two_ligatures_same_first() -> Face {
    let chars = [('a', 1u16), ('b', 2u16), ('c', 3u16)];
    let ligs = [
        // Longer ligature listed first (spec ordering rule).
        LigatureRecord {
            lig_glyph: 5,
            components: &[2u16, 3u16],
        },
        LigatureRecord {
            lig_glyph: 4,
            components: &[2u16],
        },
    ];
    let sets = [LigatureSet {
        first: 1,
        ligatures: &ligs,
    }];
    let bytes = build_synthetic_ttf(6, &chars, &sets);
    Face::from_ttf_bytes(bytes).expect("synthetic TTF parses")
}

#[test]
fn liga_longest_match_first_picks_3_glyph_ligature() {
    // "abc" should collapse to the longer ligature (gid 5), not
    // first match the 2-glyph one (gid 4).
    let face = synth_two_ligatures_same_first();
    let out = face.shape_text("abc", &[*b"liga"]);
    assert_eq!(
        out,
        vec![5u16],
        "the 3-glyph ligature wins over the 2-glyph ligature on the same first component"
    );
}

#[test]
fn liga_longest_match_first_falls_back_when_tail_missing() {
    // "ab" doesn't satisfy the 3-glyph 'abc' ligature; the
    // walker must fall back to the 2-glyph 'ab' record → [4].
    let face = synth_two_ligatures_same_first();
    let out = face.shape_text("ab", &[*b"liga"]);
    assert_eq!(out, vec![4u16]);
}

// ---------------------------------------------------------------------------
// Multiple LigatureSets — coverage routes by first glyph
// ---------------------------------------------------------------------------

/// Build a face with two LigatureSets:
/// - gid 1 ('a') + gid 2 ('b') → gid 5
/// - gid 3 ('c') + gid 4 ('d') → gid 6
fn synth_two_sets() -> Face {
    let chars = [('a', 1u16), ('b', 2u16), ('c', 3u16), ('d', 4u16)];
    let set_a = [LigatureRecord {
        lig_glyph: 5,
        components: &[2u16],
    }];
    let set_c = [LigatureRecord {
        lig_glyph: 6,
        components: &[4u16],
    }];
    let sets = [
        LigatureSet {
            first: 1,
            ligatures: &set_a,
        },
        LigatureSet {
            first: 3,
            ligatures: &set_c,
        },
    ];
    let bytes = build_synthetic_ttf(7, &chars, &sets);
    Face::from_ttf_bytes(bytes).expect("synthetic TTF parses")
}

#[test]
fn liga_two_sets_each_fires_independently() {
    let face = synth_two_sets();
    // 'ab' → gid 5; 'cd' → gid 6; mixed input "abcd" → [5, 6].
    let out = face.shape_text("abcd", &[*b"liga"]);
    assert_eq!(out, vec![5u16, 6u16]);
}

#[test]
fn liga_two_sets_first_set_only_on_partial_input() {
    let face = synth_two_sets();
    // 'ab' alone → [5]; 'cd' alone → [6].
    assert_eq!(face.shape_text("ab", &[*b"liga"]), vec![5u16]);
    assert_eq!(face.shape_text("cd", &[*b"liga"]), vec![6u16]);
}

// ---------------------------------------------------------------------------
// Single-component ligature edge case (spec-legal but rare).
// ---------------------------------------------------------------------------

/// Build a face where gid 1's LigatureSet has a degenerate 1-component
/// record (componentCount = 1, no trailing components). Per the spec
/// this is legal — effectively a Single Substitution dressed as a
/// ligature. The walker must collapse the single covered glyph to
/// the replacement and advance.
fn synth_single_component_ligature() -> Face {
    let chars = [('a', 1u16), ('b', 2u16)];
    let ligs = [LigatureRecord {
        lig_glyph: 3,
        components: &[],
    }];
    let sets = [LigatureSet {
        first: 1,
        ligatures: &ligs,
    }];
    let bytes = build_synthetic_ttf(4, &chars, &sets);
    Face::from_ttf_bytes(bytes).expect("synthetic TTF parses")
}

#[test]
fn liga_single_component_record_behaves_like_single_subst() {
    let face = synth_single_component_ligature();
    let out = face.shape_text("ab", &[*b"liga"]);
    assert_eq!(
        out,
        vec![3u16, 2u16],
        "a single-component ligature replaces the covered glyph and leaves the rest"
    );
}
