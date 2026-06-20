//! Round 353 — GSUB contextual (LookupType 5), chained-contextual
//! (LookupType 6), and reverse-chaining contextual single (LookupType
//! 8) substitution wired into the **caller-driven** `Face::shape_text`
//! surface.
//!
//! Before this round, `shaping::feature_subst::shape_text_inner` (the
//! body behind `Face::shape_text` / `shape_text_with_script` / the
//! alternate-index variants) dispatched only LookupTypes 1 / 2 / 3 / 4
//! and **silently skipped** every contextual lookup a requested feature
//! referenced. The always-on `ccmp` / `calt` passes in
//! `shaping::general` already handled types 5 / 6 / 8, so the
//! auto-probe `Shaper::shape` path was complete — but a caller that
//! explicitly asked for, say, `calt` or `frac` through `shape_text`
//! got a contextual no-op.
//!
//! ## Why synthetic fixtures
//!
//! Inter Variable does ship type-5 and type-6 GSUB lookups, but the
//! features that reference them and the exact glyph contexts that
//! trigger them are font-internal and brittle to assert on. To prove
//! the dispatch deterministically we build minimal synthetic TTFs with
//! a single feature wrapping a single contextual lookup whose match
//! and replacement are fully known. The byte layouts follow the
//! OpenType GSUB chapter (`docs/text/opentype/otspec-gsub.html`,
//! ReverseChainSingleSubstFormat1) and the common-layout-tables chapter
//! (`docs/text/opentype/otspec-chapter2-common-layout-tables.html`,
//! SequenceContextFormat3 / ChainedSequenceContextFormat3 /
//! SequenceLookupRecord) transcribed from the published spec tables. No
//! external library was consulted. A real-fixture smoke test against
//! Inter rounds the suite out by confirming the new path is a
//! transparent identity when no requested feature fires.

use oxideav_scribe::Face;

const INTER: &[u8] = include_bytes!("fixtures/InterVariable.ttf");

// ---------------------------------------------------------------------------
// sfnt scaffolding helpers (shared shape with the round-125 builder).
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

/// Coverage Format 1 over a single glyph id.
fn coverage_one(gid: u16) -> Vec<u8> {
    let mut c = Vec::new();
    c.extend_from_slice(&1u16.to_be_bytes()); // coverageFormat = 1
    c.extend_from_slice(&1u16.to_be_bytes()); // glyphCount = 1
    c.extend_from_slice(&gid.to_be_bytes());
    c
}

// ---------------------------------------------------------------------------
// Glyph map: .notdef=0, the input letters map to 'a'..'g' → 1..7.
// Contextual lookups rewrite a covered glyph into the "transformed"
// glyph at GID 1+26 etc.; we keep it simple and reuse GIDs in range.
// ---------------------------------------------------------------------------

const NUM_GLYPHS: u16 = 10;

/// Map char 'a'+(n) → GID 1+n. Letters 'a'..'i' → 1..9.
fn gid_of(ch: char) -> u16 {
    1 + (ch as u16 - 'a' as u16)
}

/// Build a synthetic TTF carrying the GSUB lookup list / feature list
/// described by `gsub_body`. `gsub_body` is the fully-assembled GSUB
/// table bytes (header + script/feature/lookup lists). The remaining
/// sfnt tables are minimal boilerplate sufficient for `Face` to parse
/// and cmap 'a'..'i'.
fn build_font(gsub: Vec<u8>) -> Vec<u8> {
    // glyf: empty (all zero-length glyphs).
    let glyf: Vec<u8> = Vec::new();

    // loca: NUM_GLYPHS+1 short offsets, all zero.
    let mut loca = Vec::new();
    for _ in 0..(NUM_GLYPHS + 1) {
        loca.extend_from_slice(&0u16.to_be_bytes());
    }
    pad_to_4(&mut loca);

    // maxp 0.5.
    let mut maxp = Vec::new();
    maxp.extend_from_slice(&0x00005000u32.to_be_bytes());
    maxp.extend_from_slice(&NUM_GLYPHS.to_be_bytes());
    pad_to_4(&mut maxp);

    // head.
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
    head.extend_from_slice(&0i16.to_be_bytes()); // short loca
    head.extend_from_slice(&0i16.to_be_bytes());
    pad_to_4(&mut head);

    // hhea.
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

    // hmtx: NUM_GLYPHS records.
    let mut hmtx = Vec::new();
    for _ in 0..NUM_GLYPHS {
        hmtx.extend_from_slice(&500u16.to_be_bytes());
        hmtx.extend_from_slice(&0i16.to_be_bytes());
    }
    pad_to_4(&mut hmtx);

    // name: empty.
    let mut name = Vec::new();
    name.extend_from_slice(&0u16.to_be_bytes());
    name.extend_from_slice(&0u16.to_be_bytes());
    name.extend_from_slice(&6u16.to_be_bytes());
    pad_to_4(&mut name);

    // cmap: format 4 mapping 'a'..'i' (0x61..0x69) → GID 1..9.
    let segcount: u16 = 2;
    let segcountx2 = segcount * 2;
    let search_range = 2u16;
    let entry_selector = 0u16;
    let range_shift = segcountx2.wrapping_sub(search_range);
    let mut sub = Vec::new();
    sub.extend_from_slice(&4u16.to_be_bytes());
    let length_offset = sub.len();
    sub.extend_from_slice(&0u16.to_be_bytes()); // length (patched)
    sub.extend_from_slice(&0u16.to_be_bytes()); // language
    sub.extend_from_slice(&segcountx2.to_be_bytes());
    sub.extend_from_slice(&search_range.to_be_bytes());
    sub.extend_from_slice(&entry_selector.to_be_bytes());
    sub.extend_from_slice(&range_shift.to_be_bytes());
    sub.extend_from_slice(&0x0069u16.to_be_bytes()); // endCode 'i'
    sub.extend_from_slice(&0xFFFFu16.to_be_bytes());
    sub.extend_from_slice(&0u16.to_be_bytes()); // reservedPad
    sub.extend_from_slice(&0x0061u16.to_be_bytes()); // startCode 'a'
    sub.extend_from_slice(&0xFFFFu16.to_be_bytes());
    sub.extend_from_slice(&0xFFA0u16.to_be_bytes()); // idDelta = 1 - 0x61
    sub.extend_from_slice(&1u16.to_be_bytes());
    sub.extend_from_slice(&0u16.to_be_bytes()); // idRangeOffset
    sub.extend_from_slice(&0u16.to_be_bytes());
    let sub_len = sub.len() as u16;
    sub[length_offset..length_offset + 2].copy_from_slice(&sub_len.to_be_bytes());

    let mut cmap = Vec::new();
    cmap.extend_from_slice(&0u16.to_be_bytes());
    cmap.extend_from_slice(&1u16.to_be_bytes());
    cmap.extend_from_slice(&3u16.to_be_bytes());
    cmap.extend_from_slice(&1u16.to_be_bytes());
    cmap.extend_from_slice(&12u32.to_be_bytes()); // subtable offset
    cmap.extend_from_slice(&sub);
    pad_to_4(&mut cmap);

    let mut gsub = gsub;
    pad_to_4(&mut gsub);

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
        let checksum = table_checksum(data);
        let length = data.len() as u32;
        out.extend_from_slice(data);
        pad_to_4(&mut out);
        records.push((tag, checksum, length, offset));
    }
    let mut rec_pos = header_size;
    for (tag, checksum, length, offset) in records {
        out[rec_pos..rec_pos + 4].copy_from_slice(tag);
        out[rec_pos + 4..rec_pos + 8].copy_from_slice(&checksum.to_be_bytes());
        out[rec_pos + 8..rec_pos + 12].copy_from_slice(&offset.to_be_bytes());
        out[rec_pos + 12..rec_pos + 16].copy_from_slice(&length.to_be_bytes());
        rec_pos += record_size;
    }
    out
}

// ---------------------------------------------------------------------------
// GSUB assembly: a script/feature/lookup-list wrapper around a single
// feature tag and a caller-supplied list of lookup subtables.
// ---------------------------------------------------------------------------

/// A single GSUB lookup: declared type + one subtable's bytes.
struct Lookup {
    ty: u16,
    subtable: Vec<u8>,
}

/// Assemble a complete GSUB table: one script (`DFLT`), one feature
/// (`feature_tag`) referencing `feature_lookups` (indices into
/// `lookups`), and the full lookup list.
fn build_gsub(feature_tag: &[u8; 4], feature_lookups: &[u16], lookups: &[Lookup]) -> Vec<u8> {
    // Lookup list.
    let mut lookup_blobs: Vec<Vec<u8>> = Vec::new();
    for lk in lookups {
        let mut lookup = Vec::new();
        lookup.extend_from_slice(&lk.ty.to_be_bytes()); // lookupType
        lookup.extend_from_slice(&0u16.to_be_bytes()); // lookupFlag
        lookup.extend_from_slice(&1u16.to_be_bytes()); // subTableCount
        let subtable_offset: u16 = 8; // 2+2+2+2
        lookup.extend_from_slice(&subtable_offset.to_be_bytes());
        lookup.extend_from_slice(&lk.subtable);
        lookup_blobs.push(lookup);
    }
    let mut lookup_list = Vec::new();
    let count = lookup_blobs.len() as u16;
    lookup_list.extend_from_slice(&count.to_be_bytes());
    let mut off: u16 = 2 + 2 * count;
    let mut offsets = Vec::new();
    for blob in &lookup_blobs {
        offsets.push(off);
        off += blob.len() as u16;
    }
    for o in &offsets {
        lookup_list.extend_from_slice(&o.to_be_bytes());
    }
    for blob in &lookup_blobs {
        lookup_list.extend_from_slice(blob);
    }

    // Feature record + feature list.
    let mut feature_record = Vec::new();
    feature_record.extend_from_slice(&0u16.to_be_bytes()); // featureParamsOffset
    feature_record.extend_from_slice(&(feature_lookups.len() as u16).to_be_bytes());
    for &li in feature_lookups {
        feature_record.extend_from_slice(&li.to_be_bytes());
    }
    let mut feature_list = Vec::new();
    feature_list.extend_from_slice(&1u16.to_be_bytes()); // featureCount
    feature_list.extend_from_slice(feature_tag);
    let feature_offset: u16 = 2 + 6;
    feature_list.extend_from_slice(&feature_offset.to_be_bytes());
    feature_list.extend_from_slice(&feature_record);

    // LangSys + Script + ScriptList (DFLT).
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
    let script_offset: u16 = 2 + 6;
    script_list.extend_from_slice(&script_offset.to_be_bytes());
    script_list.extend_from_slice(&script);

    // GSUB header.
    let header_len: u16 = 10;
    let script_list_off = header_len;
    let feature_list_off = script_list_off + script_list.len() as u16;
    let lookup_list_off = feature_list_off + feature_list.len() as u16;
    let mut gsub = Vec::new();
    gsub.extend_from_slice(&1u16.to_be_bytes());
    gsub.extend_from_slice(&0u16.to_be_bytes());
    gsub.extend_from_slice(&script_list_off.to_be_bytes());
    gsub.extend_from_slice(&feature_list_off.to_be_bytes());
    gsub.extend_from_slice(&lookup_list_off.to_be_bytes());
    gsub.extend_from_slice(&script_list);
    gsub.extend_from_slice(&feature_list);
    gsub.extend_from_slice(&lookup_list);
    gsub
}

// ---------------------------------------------------------------------------
// Subtable builders.
// ---------------------------------------------------------------------------

/// SingleSubstFormat2 over one glyph: `from` → `to`.
fn single_subst(from: u16, to: u16) -> Vec<u8> {
    let mut s = Vec::new();
    s.extend_from_slice(&2u16.to_be_bytes()); // substFormat = 2
    let cov_off: u16 = 8; // header is 2+2+2+2 (fmt, covOff, glyphCount, sub[0])
    s.extend_from_slice(&cov_off.to_be_bytes());
    s.extend_from_slice(&1u16.to_be_bytes()); // glyphCount
    s.extend_from_slice(&to.to_be_bytes()); // substituteGlyphIDs[0]
    s.extend_from_slice(&coverage_one(from));
    s
}

/// SequenceContextFormat3 (GSUB type 5): input coverage sequence
/// `input_gids`, applying `seq_lookups` (sequenceIndex, lookupListIndex).
fn context_format3(input_gids: &[u16], seq_lookups: &[(u16, u16)]) -> Vec<u8> {
    let glyph_count = input_gids.len() as u16;
    let seq_count = seq_lookups.len() as u16;
    // Header: format(2) + glyphCount(2) + seqLookupCount(2)
    //         + coverageOffsets[glyphCount]*2 + seqLookupRecords*4
    let header_len = 6 + 2 * input_gids.len() + 4 * seq_lookups.len();
    let mut s = Vec::new();
    s.extend_from_slice(&3u16.to_be_bytes()); // format
    s.extend_from_slice(&glyph_count.to_be_bytes());
    s.extend_from_slice(&seq_count.to_be_bytes());
    // Coverage tables placed after the seqLookupRecords.
    let mut cov_off = header_len as u16;
    let mut cov_blobs = Vec::new();
    for &g in input_gids {
        s.extend_from_slice(&cov_off.to_be_bytes());
        let blob = coverage_one(g);
        cov_off += blob.len() as u16;
        cov_blobs.push(blob);
    }
    for &(si, li) in seq_lookups {
        s.extend_from_slice(&si.to_be_bytes());
        s.extend_from_slice(&li.to_be_bytes());
    }
    for blob in cov_blobs {
        s.extend_from_slice(&blob);
    }
    s
}

/// ChainedSequenceContextFormat3 (GSUB type 6).
fn chained_format3(
    backtrack: &[u16],
    input: &[u16],
    lookahead: &[u16],
    seq_lookups: &[(u16, u16)],
) -> Vec<u8> {
    // Layout:
    //   format(2)
    //   backtrackGlyphCount(2) + backtrackCoverageOffsets[]*2
    //   inputGlyphCount(2)     + inputCoverageOffsets[]*2
    //   lookaheadGlyphCount(2) + lookaheadCoverageOffsets[]*2
    //   seqLookupCount(2)      + seqLookupRecords*4
    //   <coverage tables>
    let header_len = 2
        + 2
        + 2 * backtrack.len()
        + 2
        + 2 * input.len()
        + 2
        + 2 * lookahead.len()
        + 2
        + 4 * seq_lookups.len();
    let mut s = Vec::new();
    let mut cov_off = header_len as u16;
    let mut cov_blobs: Vec<Vec<u8>> = Vec::new();
    let push_seq = |s: &mut Vec<u8>, seq: &[u16], cov_off: &mut u16, blobs: &mut Vec<Vec<u8>>| {
        s.extend_from_slice(&(seq.len() as u16).to_be_bytes());
        for &g in seq {
            s.extend_from_slice(&cov_off.to_be_bytes());
            let blob = coverage_one(g);
            *cov_off += blob.len() as u16;
            blobs.push(blob);
        }
    };
    s.extend_from_slice(&3u16.to_be_bytes()); // format
    push_seq(&mut s, backtrack, &mut cov_off, &mut cov_blobs);
    push_seq(&mut s, input, &mut cov_off, &mut cov_blobs);
    push_seq(&mut s, lookahead, &mut cov_off, &mut cov_blobs);
    s.extend_from_slice(&(seq_lookups.len() as u16).to_be_bytes());
    for &(si, li) in seq_lookups {
        s.extend_from_slice(&si.to_be_bytes());
        s.extend_from_slice(&li.to_be_bytes());
    }
    for blob in cov_blobs {
        s.extend_from_slice(&blob);
    }
    s
}

/// ReverseChainSingleSubstFormat1 (GSUB type 8): cover `from`, with the
/// given `lookahead` coverage sequence, substituting `from` → `to`.
fn reverse_chain(from: u16, lookahead: &[u16], to: u16) -> Vec<u8> {
    // Layout:
    //   format(2) + coverageOffset(2)
    //   backtrackGlyphCount(2) + backtrackCoverageOffsets[]*2
    //   lookaheadGlyphCount(2) + lookaheadCoverageOffsets[]*2
    //   glyphCount(2) + substituteGlyphIDs[]*2
    //   <coverage tables>
    let header_len = 2 + 2 + 2 + 2 + 2 * lookahead.len() + 2 + 2;
    let mut s = Vec::new();
    let mut cov_off = header_len as u16;
    s.extend_from_slice(&1u16.to_be_bytes()); // format
    let input_cov_off_pos = s.len();
    s.extend_from_slice(&0u16.to_be_bytes()); // coverageOffset (patched)
    s.extend_from_slice(&0u16.to_be_bytes()); // backtrackGlyphCount
    s.extend_from_slice(&(lookahead.len() as u16).to_be_bytes()); // lookaheadGlyphCount
    let mut la_blobs = Vec::new();
    for &g in lookahead {
        s.extend_from_slice(&cov_off.to_be_bytes());
        let blob = coverage_one(g);
        cov_off += blob.len() as u16;
        la_blobs.push(blob);
    }
    s.extend_from_slice(&1u16.to_be_bytes()); // glyphCount
    s.extend_from_slice(&to.to_be_bytes()); // substituteGlyphIDs[0]
                                            // Coverage tables are appended in the same order their offsets were
                                            // assigned: the lookahead coverages start at `header_len`, the input
                                            // coverage follows at `cov_off` (its post-lookahead value).
    let input_cov_off = cov_off;
    s[input_cov_off_pos..input_cov_off_pos + 2].copy_from_slice(&input_cov_off.to_be_bytes());
    for blob in la_blobs {
        s.extend_from_slice(&blob);
    }
    s.extend_from_slice(&coverage_one(from));
    s
}

// ---------------------------------------------------------------------------
// Tests — GSUB LookupType 5 (Contextual).
// ---------------------------------------------------------------------------

/// Context "a b" → apply single-subst (a→i) at input position 0. The
/// caller-driven path must now fire this; "ab" → [gid_i, gid_b].
#[test]
fn type5_contextual_fires_via_shape_text() {
    let a = gid_of('a');
    let b = gid_of('b');
    let i = gid_of('i');
    let lookups = vec![
        Lookup {
            ty: 1,
            subtable: single_subst(a, i),
        },
        Lookup {
            ty: 5,
            subtable: context_format3(&[a, b], &[(0, 0)]),
        },
    ];
    // Feature references the type-5 lookup (index 1) only.
    let gsub = build_gsub(b"test", &[1], &lookups);
    let face = Face::from_ttf_bytes(build_font(gsub)).expect("synthetic font parses");

    // "ab": context matches at pos 0 → a→i. Output [i, b].
    let out = face.shape_text("ab", &[*b"test"]);
    assert_eq!(out, vec![i, b], "type-5 contextual subst did not fire");

    // "ba": context [a,b] does not start at pos 0 (b then a) → identity.
    let out2 = face.shape_text("ba", &[*b"test"]);
    assert_eq!(out2, vec![b, a], "type-5 fired on a non-matching context");

    // Bare "a" (no following b) → context cannot match → identity.
    let out3 = face.shape_text("a", &[*b"test"]);
    assert_eq!(out3, vec![a], "type-5 fired without its required context");
}

/// Without requesting the feature, the contextual lookup must not fire —
/// `shape_text` with empty features is pure cmap.
#[test]
fn type5_inert_without_feature() {
    let a = gid_of('a');
    let b = gid_of('b');
    let i = gid_of('i');
    let lookups = vec![
        Lookup {
            ty: 1,
            subtable: single_subst(a, i),
        },
        Lookup {
            ty: 5,
            subtable: context_format3(&[a, b], &[(0, 0)]),
        },
    ];
    let gsub = build_gsub(b"test", &[1], &lookups);
    let face = Face::from_ttf_bytes(build_font(gsub)).expect("synthetic font parses");
    assert_eq!(face.shape_text("ab", &[]), vec![a, b]);
}

// ---------------------------------------------------------------------------
// Tests — GSUB LookupType 6 (Chained Contexts).
// ---------------------------------------------------------------------------

/// Chained context: backtrack [c], input [a], lookahead [d]. So
/// "c a d" → "c i d" (a→i), but bare "a" and "a d" (no backtrack) stay.
#[test]
fn type6_chained_context_fires_via_shape_text() {
    let a = gid_of('a');
    let c = gid_of('c');
    let d = gid_of('d');
    let i = gid_of('i');
    let lookups = vec![
        Lookup {
            ty: 1,
            subtable: single_subst(a, i),
        },
        Lookup {
            ty: 6,
            subtable: chained_format3(&[c], &[a], &[d], &[(0, 0)]),
        },
    ];
    let gsub = build_gsub(b"tes2", &[1], &lookups);
    let face = Face::from_ttf_bytes(build_font(gsub)).expect("synthetic font parses");

    // "cad": full context → a→i. Output [c, i, d].
    assert_eq!(
        face.shape_text("cad", &[*b"tes2"]),
        vec![c, i, d],
        "type-6 chained context did not fire on a full match"
    );

    // "ad": missing backtrack 'c' → identity.
    assert_eq!(
        face.shape_text("ad", &[*b"tes2"]),
        vec![a, d],
        "type-6 fired without its backtrack context"
    );

    // "ca": missing lookahead 'd' → identity.
    assert_eq!(
        face.shape_text("ca", &[*b"tes2"]),
        vec![c, a],
        "type-6 fired without its lookahead context"
    );
}

// ---------------------------------------------------------------------------
// Tests — GSUB LookupType 8 (Reverse Chaining Contextual Single).
// ---------------------------------------------------------------------------

/// Reverse chain: cover 'a', lookahead [d], substitute a→i. So "a d" →
/// "i d", bare "a" stays.
#[test]
fn type8_reverse_chain_fires_via_shape_text() {
    let a = gid_of('a');
    let d = gid_of('d');
    let i = gid_of('i');
    let lookups = vec![Lookup {
        ty: 8,
        subtable: reverse_chain(a, &[d], i),
    }];
    let gsub = build_gsub(b"tes3", &[0], &lookups);
    let face = Face::from_ttf_bytes(build_font(gsub)).expect("synthetic font parses");

    assert_eq!(
        face.shape_text("ad", &[*b"tes3"]),
        vec![i, d],
        "type-8 reverse chain did not fire with its lookahead present"
    );
    assert_eq!(
        face.shape_text("a", &[*b"tes3"]),
        vec![a],
        "type-8 fired without its lookahead context"
    );
}

/// Reverse-processing correctness: cover 'a', lookahead [a], a→i. The
/// run "aaa" must process right-to-left. Right-to-left: the rightmost
/// 'a' (index 2) has no lookahead 'a' → stays 'a'. Index 1 sees index 2
/// = 'a' as lookahead → becomes 'i'. Index 0 sees index 1 — but by the
/// time index 0 is processed, index 1 is already 'i' (not 'a'), so the
/// lookahead 'a' no longer matches → index 0 stays 'a'. A buggy
/// left-to-right walk would instead turn index 0 → 'i' (it still sees a
/// raw 'a' at index 1) and produce a different result.
#[test]
fn type8_processes_right_to_left() {
    let a = gid_of('a');
    let i = gid_of('i');
    let lookups = vec![Lookup {
        ty: 8,
        subtable: reverse_chain(a, &[a], i),
    }];
    let gsub = build_gsub(b"tes4", &[0], &lookups);
    let face = Face::from_ttf_bytes(build_font(gsub)).expect("synthetic font parses");

    // Right-to-left: [a, a, a] → [a, i, a].
    assert_eq!(
        face.shape_text("aaa", &[*b"tes4"]),
        vec![a, i, a],
        "type-8 did not process the run right-to-left"
    );
}

// ---------------------------------------------------------------------------
// Real-fixture smoke: the new path is a transparent identity when no
// requested feature drives a contextual lookup over the run.
// ---------------------------------------------------------------------------

/// Inter ships type-5/6 GSUB lookups. Shaping a plain Latin word with a
/// feature it doesn't publish for the run must equal the cmap output —
/// the contextual dispatch must not corrupt a run it shouldn't touch.
#[test]
fn inter_unrequested_feature_is_identity() {
    let face = Face::from_ttf_bytes(INTER.to_vec()).expect("Inter parses");
    let cmap_only = face.shape_text("Hello", &[]);
    // A feature tag Inter does not publish under latn for this run.
    let with_bogus = face.shape_text("Hello", &[*b"zzzz"]);
    assert_eq!(
        cmap_only, with_bogus,
        "unrequested/absent feature changed the run"
    );
}

/// Confirm Inter genuinely ships contextual GSUB lookups so the
/// dispatch additions are exercised by a real font's lookup list (not
/// just synthetic fixtures).
#[test]
fn inter_ships_contextual_gsub_lookups() {
    let face = Face::from_ttf_bytes(INTER.to_vec()).expect("Inter parses");
    face.with_font(|font| {
        let ctx = font
            .gsub_lookup_list()
            .iter()
            .filter(|&&(_, ty, _)| ty == 5 || ty == 6 || ty == 8)
            .count();
        assert!(
            ctx > 0,
            "Inter unexpectedly ships no contextual GSUB lookups"
        );
    })
    .unwrap();
}
