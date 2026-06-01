//! Variable-font metrics + style-attribute tables that scribe parses
//! locally — `MVAR`, `HVAR`, `VVAR`, `STAT`, plus `name` table id
//! resolution and the CFF2 INDEX walker.
//!
//! Scope for these tables (added in #454):
//!
//! - **MVAR** — global metric variations (cap-height, x-height,
//!   ascender, descender, …) keyed by 4-byte ValueTag.
//! - **HVAR / VVAR** — per-glyph horizontal- / vertical-advance and
//!   side-bearing variations keyed by glyph index.
//! - **STAT** — design-axis labelling (e.g. `wght 400 → "Regular"`),
//!   surfaced as metadata for downstream callers.
//! - **`name` resolver** — given a `name`-table id, return the
//!   highest-ranked Unicode string (Windows English first, Mac Roman
//!   English next, anything Unicode-y after, then any remaining
//!   record). Mirrors the priority used by the underlying
//!   `oxideav_ttf::tables::name::NameTable` but exposed at the
//!   higher-level `Face` so callers can resolve `axis_name_id` /
//!   `subfamily_name_id` / `value_name_id` (from STAT) without
//!   reaching into the inner parser.
//! - **CFF2 INDEX walker** — minimal CFF2 table support: header
//!   (5 bytes: major, minor, headerSize, topDictLength) + Name +
//!   Top DICT + String + Global Subr INDEX layout walk plus per-axis
//!   variation-store offset extraction. Full charstring v3 evaluation
//!   under variations is out of scope for this round; the walker
//!   exposes table presence + axis count + glyph count so callers can
//!   confirm a CFF2 font parses, and the metrics-variation pipeline
//!   (MVAR / HVAR) keeps working on top of CFF2 fonts exactly as on
//!   TT-flavour ones.
//!
//! All tables are parsed from raw font bytes obtained via
//! [`Face::with_font`] / [`Face::with_otf_font`] (both expose
//! `Font::bytes()` on the inner parser). We deliberately avoid adding
//! new parsers to `oxideav-ttf` / `oxideav-otf` so this round can ship
//! independently of the producer crates' release cadence (per the
//! workspace publish-before-consume policy).
//!
//! Spec references:
//! - Microsoft OpenType §"MVAR — Metrics Variations Table".
//! - Microsoft OpenType §"HVAR — Horizontal Metrics Variations Table".
//! - Microsoft OpenType §"VVAR — Vertical Metrics Variations Table".
//! - Microsoft OpenType §"STAT — Style Attributes Table".
//! - Adobe Technical Note #5176 + §"CFF2 charstring format" (TN5177).
//! - Microsoft OpenType §"Item Variation Store Header and Item
//!   Variation Subtables".
//! - Microsoft OpenType §"Delta Set Index Map Table".

use core::fmt::{self, Debug};

// ---------------------------------------------------------------------
// sfnt table directory walker — the smallest possible re-implementation,
// enough to look up a single 4-byte tag in raw font bytes.
// ---------------------------------------------------------------------

const SFNT_TT: u32 = 0x0001_0000;
const SFNT_TRUE: u32 = 0x74727565; // 'true'
const SFNT_OTTO: u32 = 0x4F54_544F; // 'OTTO'

/// Locate the bytes of `tag` inside an sfnt-flavoured font, starting at
/// the optional `header_offset` (use 0 for plain sfnt; non-zero for a
/// TTC subfont). Returns `None` if the magic is unrecognised, the
/// table is missing, or any offset/length pair points past the end.
pub fn find_table<'a>(bytes: &'a [u8], tag: &[u8; 4], header_offset: usize) -> Option<&'a [u8]> {
    if bytes.len() < header_offset + 12 {
        return None;
    }
    let h = &bytes[header_offset..];
    let magic = u32::from_be_bytes([h[0], h[1], h[2], h[3]]);
    if magic != SFNT_TT && magic != SFNT_TRUE && magic != SFNT_OTTO {
        return None;
    }
    let n = u16::from_be_bytes([h[4], h[5]]) as usize;
    if h.len() < 12 + n * 16 {
        return None;
    }
    for i in 0..n {
        let rec = &h[12 + i * 16..12 + (i + 1) * 16];
        if &rec[0..4] == tag {
            let off = u32::from_be_bytes([rec[8], rec[9], rec[10], rec[11]]) as usize;
            let len = u32::from_be_bytes([rec[12], rec[13], rec[14], rec[15]]) as usize;
            if off.saturating_add(len) > bytes.len() {
                return None;
            }
            return Some(&bytes[off..off + len]);
        }
    }
    None
}

// ---------------------------------------------------------------------
// Tiny BE readers (private — match the ones in oxideav-ttf::parser).
// ---------------------------------------------------------------------

#[inline]
fn u16_be(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(o)?, *b.get(o + 1)?]))
}
#[inline]
fn i16_be(b: &[u8], o: usize) -> Option<i16> {
    Some(i16::from_be_bytes([*b.get(o)?, *b.get(o + 1)?]))
}
#[inline]
fn u32_be(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *b.get(o)?,
        *b.get(o + 1)?,
        *b.get(o + 2)?,
        *b.get(o + 3)?,
    ]))
}
#[inline]
fn i32_be(b: &[u8], o: usize) -> Option<i32> {
    Some(i32::from_be_bytes([
        *b.get(o)?,
        *b.get(o + 1)?,
        *b.get(o + 2)?,
        *b.get(o + 3)?,
    ]))
}
#[inline]
fn f2dot14(raw: i16) -> f32 {
    raw as f32 / 16384.0
}

// ---------------------------------------------------------------------
// ItemVariationStore — shared by MVAR / HVAR / VVAR.
// ---------------------------------------------------------------------

/// One variation region — a per-axis `(start, peak, end)` box in
/// normalised coordinates.
#[derive(Debug, Clone)]
struct Region {
    /// `(start, peak, end)` per axis, F2DOT14 → f32.
    axes: Vec<(f32, f32, f32)>,
}

impl Region {
    /// Per OpenType §"Calculation of Item Variation Data Scalar":
    ///
    /// 1. If `peak == 0` for this axis, contribution is 1.0.
    /// 2. Else if the axis position is `0`, OR has opposite sign to
    ///    `peak`, contribution is 0.0.
    /// 3. Else if position equals peak, contribution is 1.0.
    /// 4. Else if position is between `start` and `peak`, ramp up
    ///    linearly from 0 to 1.
    /// 5. Else if position is between `peak` and `end`, ramp down
    ///    linearly from 1 to 0.
    /// 6. Else (outside `[start, end]`), contribution is 0.0.
    ///
    /// Per-axis contributions multiply.
    fn scalar(&self, coords: &[f32]) -> f32 {
        let mut s = 1.0f32;
        for (ai, &(start, peak, end)) in self.axes.iter().enumerate() {
            let c = coords.get(ai).copied().unwrap_or(0.0);
            if peak == 0.0 {
                continue;
            }
            if c == 0.0 || (c < 0.0) != (peak < 0.0) {
                return 0.0;
            }
            if c == peak {
                continue;
            }
            if c < start || c > end {
                return 0.0;
            }
            if c < peak {
                if (peak - start).abs() < f32::EPSILON {
                    return 0.0;
                }
                s *= (c - start) / (peak - start);
            } else {
                if (end - peak).abs() < f32::EPSILON {
                    return 0.0;
                }
                s *= (end - c) / (end - peak);
            }
        }
        s
    }
}

/// One ItemVariationData subtable.
#[derive(Debug, Clone)]
struct ItemVariationData {
    /// Indexes into the parent ItemVariationStore's region list.
    region_indexes: Vec<u16>,
    /// `delta_sets[item][region]` — i32 to fit either i16 or i8 deltas
    /// (and i32 in the long-mode variant).
    delta_sets: Vec<Vec<i32>>,
}

/// Parsed ItemVariationStore.
#[derive(Debug, Clone, Default)]
pub struct ItemVariationStore {
    axis_count: u16,
    regions: Vec<Region>,
    subtables: Vec<ItemVariationData>,
}

impl ItemVariationStore {
    /// Parse an ItemVariationStore that starts at `offset` inside `bytes`.
    pub fn parse(bytes: &[u8], offset: usize) -> Option<Self> {
        let b = bytes.get(offset..)?;
        let format = u16_be(b, 0)?;
        if format != 1 {
            return None;
        }
        let region_list_offset = u32_be(b, 2)? as usize;
        let item_var_data_count = u16_be(b, 6)? as usize;

        // VariationRegionList.
        let r = b.get(region_list_offset..)?;
        let axis_count = u16_be(r, 0)?;
        let region_count = u16_be(r, 2)? as usize;
        let mut regions = Vec::with_capacity(region_count);
        let region_size = axis_count as usize * 6; // 3 × F2DOT14
        for ri in 0..region_count {
            let off = 4 + ri * region_size;
            let rb = r.get(off..off + region_size)?;
            let mut axes = Vec::with_capacity(axis_count as usize);
            for ai in 0..axis_count as usize {
                let start = f2dot14(i16_be(rb, ai * 6)?);
                let peak = f2dot14(i16_be(rb, ai * 6 + 2)?);
                let end = f2dot14(i16_be(rb, ai * 6 + 4)?);
                axes.push((start, peak, end));
            }
            regions.push(Region { axes });
        }

        // ItemVariationData subtables.
        let mut subtables = Vec::with_capacity(item_var_data_count);
        for ii in 0..item_var_data_count {
            let sub_off_off = 8 + ii * 4;
            let sub_off = u32_be(b, sub_off_off)? as usize;
            let sb = b.get(sub_off..)?;
            let item_count = u16_be(sb, 0)? as usize;
            let raw_word_delta_count = u16_be(sb, 2)?;
            let long_mode = (raw_word_delta_count & 0x8000) != 0;
            let word_delta_count = (raw_word_delta_count & 0x7FFF) as usize;
            let region_index_count = u16_be(sb, 4)? as usize;
            let mut region_indexes = Vec::with_capacity(region_index_count);
            for k in 0..region_index_count {
                region_indexes.push(u16_be(sb, 6 + k * 2)?);
            }
            if word_delta_count > region_index_count {
                return None;
            }
            let row_size = if long_mode {
                word_delta_count * 4 + (region_index_count - word_delta_count) * 2
            } else {
                word_delta_count * 2 + (region_index_count - word_delta_count)
            };
            let row_base = 6 + region_index_count * 2;
            let mut delta_sets = Vec::with_capacity(item_count);
            for it in 0..item_count {
                let off = row_base + it * row_size;
                let rs = sb.get(off..off + row_size)?;
                let mut deltas = Vec::with_capacity(region_index_count);
                let mut rp = 0usize;
                for k in 0..region_index_count {
                    if k < word_delta_count {
                        if long_mode {
                            deltas.push(i32_be(rs, rp)?);
                            rp += 4;
                        } else {
                            deltas.push(i16_be(rs, rp)? as i32);
                            rp += 2;
                        }
                    } else if long_mode {
                        deltas.push(i16_be(rs, rp)? as i32);
                        rp += 2;
                    } else {
                        deltas.push((rs[rp] as i8) as i32);
                        rp += 1;
                    }
                }
                delta_sets.push(deltas);
            }
            subtables.push(ItemVariationData {
                region_indexes,
                delta_sets,
            });
        }

        Some(Self {
            axis_count,
            regions,
            subtables,
        })
    }

    /// Per-axis count the store was built against.
    pub fn axis_count(&self) -> u16 {
        self.axis_count
    }

    /// Resolve `(outer_index, inner_index)` to a scaled delta in font
    /// units at the given normalised coords. Returns `0` for an
    /// out-of-range index pair (matching the spec's "absent → 0" rule).
    pub fn resolve_delta(&self, outer: u16, inner: u16, coords: &[f32]) -> f32 {
        let st = match self.subtables.get(outer as usize) {
            Some(s) => s,
            None => return 0.0,
        };
        let row = match st.delta_sets.get(inner as usize) {
            Some(r) => r,
            None => return 0.0,
        };
        let mut sum = 0.0f32;
        for (k, &d) in row.iter().enumerate() {
            let region_index = match st.region_indexes.get(k) {
                Some(&i) => i as usize,
                None => continue,
            };
            let region = match self.regions.get(region_index) {
                Some(r) => r,
                None => continue,
            };
            let s = region.scalar(coords);
            if s != 0.0 {
                sum += s * d as f32;
            }
        }
        sum
    }
}

// ---------------------------------------------------------------------
// DeltaSetIndexMap — used by HVAR / VVAR.
// ---------------------------------------------------------------------

/// Parsed DeltaSetIndexMap.
#[derive(Debug, Clone)]
pub struct DeltaSetIndexMap {
    inner_bits: u32,
    map_count: u32,
    /// Each entry is a pre-resolved `(outer, inner)` index pair.
    entries: Vec<(u16, u16)>,
}

impl DeltaSetIndexMap {
    /// Parse a DeltaSetIndexMap that starts at `offset` inside `bytes`.
    pub fn parse(bytes: &[u8], offset: usize) -> Option<Self> {
        let b = bytes.get(offset..)?;
        if b.len() < 4 {
            return None;
        }
        let format = b[0];
        let entry_format = b[1];
        let (map_count, header_size) = match format {
            0 => (u16_be(b, 2)? as u32, 4usize),
            1 => (u32_be(b, 2)?, 6usize),
            _ => return None,
        };
        let entry_size = (((entry_format >> 4) & 0x03) + 1) as usize;
        let inner_bits = ((entry_format & 0x0F) + 1) as u32;
        let inner_mask = (1u32 << inner_bits) - 1;
        let need = header_size + entry_size * map_count as usize;
        if b.len() < need {
            return None;
        }
        let mut entries = Vec::with_capacity(map_count as usize);
        for i in 0..map_count as usize {
            let p = header_size + i * entry_size;
            let mut v = 0u32;
            for k in 0..entry_size {
                v = (v << 8) | b[p + k] as u32;
            }
            let inner = (v & inner_mask) as u16;
            let outer = ((v >> inner_bits) & 0xFFFF) as u16;
            entries.push((outer, inner));
        }
        Some(Self {
            inner_bits,
            map_count,
            entries,
        })
    }

    /// Resolve an item key (e.g. glyph id) to `(outer, inner)`.
    /// If `key >= map_count` the spec says use the LAST entry.
    pub fn resolve(&self, key: u32) -> (u16, u16) {
        if self.entries.is_empty() {
            return (0, 0);
        }
        let idx = if key >= self.map_count {
            self.entries.len() - 1
        } else {
            key as usize
        };
        self.entries[idx]
    }

    /// Number of mapped entries.
    pub fn map_count(&self) -> u32 {
        self.map_count
    }

    /// Bit count used for the inner index in each entry.
    pub fn inner_bits(&self) -> u32 {
        self.inner_bits
    }
}

// ---------------------------------------------------------------------
// MVAR — Metrics Variations.
// ---------------------------------------------------------------------

/// One MVAR ValueRecord (4-byte tag → outer+inner index pair).
#[derive(Debug, Clone)]
pub struct MvarValue {
    pub tag: [u8; 4],
    pub outer: u16,
    pub inner: u16,
}

/// Parsed MVAR table.
#[derive(Debug, Clone)]
pub struct MvarTable {
    values: Vec<MvarValue>,
    ivs: Option<ItemVariationStore>,
}

impl MvarTable {
    /// Parse the raw bytes of an MVAR table.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 12 {
            return None;
        }
        let major = u16_be(bytes, 0)?;
        if major != 1 {
            return None;
        }
        // bytes[2..4] minor, [4..6] reserved
        let value_record_size = u16_be(bytes, 6)? as usize;
        let value_record_count = u16_be(bytes, 8)? as usize;
        let ivs_offset = u16_be(bytes, 10)? as usize;

        let header_size = 12usize;
        let need = header_size + value_record_count * value_record_size;
        if bytes.len() < need {
            return None;
        }
        // ValueRecord layout: tag(4) + outer(2) + inner(2). The size
        // field is reserved-for-future-extension; we tolerate >8 by
        // stride-skipping the trailing bytes.
        if value_record_size < 8 {
            return None;
        }
        let mut values = Vec::with_capacity(value_record_count);
        for i in 0..value_record_count {
            let off = header_size + i * value_record_size;
            let mut tag = [0u8; 4];
            tag.copy_from_slice(&bytes[off..off + 4]);
            let outer = u16_be(bytes, off + 4)?;
            let inner = u16_be(bytes, off + 6)?;
            values.push(MvarValue { tag, outer, inner });
        }
        let ivs = if ivs_offset != 0 {
            ItemVariationStore::parse(bytes, ivs_offset)
        } else {
            None
        };
        Some(Self { values, ivs })
    }

    /// All ValueRecords in the table.
    pub fn values(&self) -> &[MvarValue] {
        &self.values
    }

    /// Compute the metric delta for `tag` at the given normalised
    /// coords. Returns `0.0` if the tag isn't enumerated or the
    /// ItemVariationStore is empty.
    pub fn delta(&self, tag: &[u8; 4], coords: &[f32]) -> f32 {
        let v = self.values.iter().find(|v| &v.tag == tag);
        let v = match v {
            Some(v) => v,
            None => return 0.0,
        };
        match self.ivs.as_ref() {
            Some(s) => s.resolve_delta(v.outer, v.inner, coords),
            None => 0.0,
        }
    }
}

// ---------------------------------------------------------------------
// HVAR / VVAR — Per-glyph advance + side-bearing variations.
// ---------------------------------------------------------------------

/// Parsed HVAR (or VVAR) table — they share the byte layout, only the
/// semantic axis (horizontal vs vertical) differs.
#[derive(Debug, Clone)]
pub struct AdvanceVariationTable {
    ivs: ItemVariationStore,
    advance_map: Option<DeltaSetIndexMap>,
    lsb_map: Option<DeltaSetIndexMap>,
    rsb_map: Option<DeltaSetIndexMap>,
}

impl AdvanceVariationTable {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 20 {
            return None;
        }
        let major = u16_be(bytes, 0)?;
        if major != 1 {
            return None;
        }
        let ivs_off = u32_be(bytes, 4)? as usize;
        let advance_map_off = u32_be(bytes, 8)? as usize;
        let lsb_map_off = u32_be(bytes, 12)? as usize;
        let rsb_map_off = u32_be(bytes, 16)? as usize;
        let ivs = ItemVariationStore::parse(bytes, ivs_off)?;
        let advance_map = if advance_map_off != 0 {
            DeltaSetIndexMap::parse(bytes, advance_map_off)
        } else {
            None
        };
        let lsb_map = if lsb_map_off != 0 {
            DeltaSetIndexMap::parse(bytes, lsb_map_off)
        } else {
            None
        };
        let rsb_map = if rsb_map_off != 0 {
            DeltaSetIndexMap::parse(bytes, rsb_map_off)
        } else {
            None
        };
        Some(Self {
            ivs,
            advance_map,
            lsb_map,
            rsb_map,
        })
    }

    /// Advance-axis delta for `gid` at the given normalised coords. For
    /// HVAR this is `horiAdvance`, for VVAR `vertAdvance`.
    ///
    /// The delta is returned in font units (callers scale by
    /// `size_px / units_per_em`).
    pub fn advance_delta(&self, gid: u16, coords: &[f32]) -> f32 {
        let (outer, inner) = match self.advance_map.as_ref() {
            Some(m) => m.resolve(gid as u32),
            // No advance map → identity: outer = 0, inner = gid.
            None => (0, gid),
        };
        self.ivs.resolve_delta(outer, inner, coords)
    }

    /// Side-bearing delta for `gid`. For HVAR this is `lsb` (the left
    /// side bearing); VVAR uses `tsb` (top side bearing). Returns 0
    /// when the LSB sub-map isn't present.
    pub fn lsb_delta(&self, gid: u16, coords: &[f32]) -> f32 {
        match self.lsb_map.as_ref() {
            Some(m) => {
                let (outer, inner) = m.resolve(gid as u32);
                self.ivs.resolve_delta(outer, inner, coords)
            }
            None => 0.0,
        }
    }

    /// Right-side-bearing delta for `gid` (HVAR's `rsb` / VVAR's `bsb`).
    /// Returns 0 when the RSB sub-map isn't present.
    pub fn rsb_delta(&self, gid: u16, coords: &[f32]) -> f32 {
        match self.rsb_map.as_ref() {
            Some(m) => {
                let (outer, inner) = m.resolve(gid as u32);
                self.ivs.resolve_delta(outer, inner, coords)
            }
            None => 0.0,
        }
    }

    /// `true` when the table publishes a non-empty advance map, OR the
    /// caller can fall back to the implicit `gid → (0, gid)` identity
    /// (which ALSO works for fonts that omit the advance map).
    pub fn has_advance_axis(&self) -> bool {
        self.advance_map.is_some() || !self.ivs.subtables.is_empty()
    }
}

// ---------------------------------------------------------------------
// STAT — Style Attributes.
// ---------------------------------------------------------------------

/// One STAT design-axis declaration.
#[derive(Debug, Clone)]
pub struct StatAxis {
    pub tag: [u8; 4],
    pub axis_name_id: u16,
    pub axis_ordering: u16,
}

/// One STAT axis-value record. The four formats from the spec map onto
/// this enum 1:1 — callers pattern-match on the variant they care about
/// (often format 1 — a single point name like `wght 400 → "Regular"`).
#[derive(Debug, Clone)]
pub enum StatAxisValue {
    /// Format 1: one named value on one axis.
    Single {
        axis_index: u16,
        flags: u16,
        value_name_id: u16,
        value: f32,
    },
    /// Format 2: a range of values on one axis (e.g. wght 600..700 →
    /// "SemiBold/Bold").
    Range {
        axis_index: u16,
        flags: u16,
        value_name_id: u16,
        nominal_value: f32,
        range_min: f32,
        range_max: f32,
    },
    /// Format 3: a named value plus a "linked" value (e.g. Italic flag
    /// linking to upright).
    Linked {
        axis_index: u16,
        flags: u16,
        value_name_id: u16,
        value: f32,
        linked_value: f32,
    },
    /// Format 4: a combined named value across multiple axes
    /// (e.g. wght 700 + wdth 75 → "Bold Condensed").
    Combined {
        flags: u16,
        value_name_id: u16,
        per_axis: Vec<(u16, f32)>,
    },
}

impl StatAxisValue {
    /// `valueNameID` for the human-readable label of this record
    /// (resolve via [`NameTableSnapshot::find`]).
    pub fn value_name_id(&self) -> u16 {
        match self {
            Self::Single { value_name_id, .. }
            | Self::Range { value_name_id, .. }
            | Self::Linked { value_name_id, .. }
            | Self::Combined { value_name_id, .. } => *value_name_id,
        }
    }
}

/// Parsed STAT table.
#[derive(Debug, Clone)]
pub struct StatTable {
    axes: Vec<StatAxis>,
    axis_values: Vec<StatAxisValue>,
    /// `name`-table id of the "elided" subfamily (typically "Regular").
    /// 0xFFFF or 2 ("Regular") in v1.0; explicit field in v1.1+.
    elided_fallback_name_id: u16,
}

impl StatTable {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        // STAT header layout (from MS OpenType spec):
        //   uint16 majorVersion         @ 0
        //   uint16 minorVersion         @ 2
        //   uint16 designAxisSize       @ 4
        //   uint16 designAxisCount      @ 6
        //   Offset32 designAxesOffset   @ 8
        //   uint16 axisValueCount       @ 12
        //   Offset32 offsetToAxisValueOffsets @ 14
        //   uint16 elidedFallbackNameID @ 18  (added in v1.1)
        // → minimum legal v1.0 = 18 bytes; v1.1 = 20 bytes.
        if bytes.len() < 18 {
            return None;
        }
        let major = u16_be(bytes, 0)?;
        if major != 1 {
            return None;
        }
        let minor = u16_be(bytes, 2)?;
        let design_axis_size = u16_be(bytes, 4)? as usize;
        let design_axis_count = u16_be(bytes, 6)? as usize;
        let design_axes_offset = u32_be(bytes, 8)? as usize;
        let axis_value_count = u16_be(bytes, 12)? as usize;
        let axis_values_offset = u32_be(bytes, 14)? as usize;
        let elided_fallback_name_id = if minor >= 1 && bytes.len() >= 20 {
            u16_be(bytes, 18)?
        } else {
            2
        };

        if design_axis_size < 8 {
            return None;
        }
        let need = design_axes_offset + design_axis_count * design_axis_size;
        if bytes.len() < need {
            return None;
        }
        let mut axes = Vec::with_capacity(design_axis_count);
        for i in 0..design_axis_count {
            let off = design_axes_offset + i * design_axis_size;
            let mut tag = [0u8; 4];
            tag.copy_from_slice(&bytes[off..off + 4]);
            let axis_name_id = u16_be(bytes, off + 4)?;
            let axis_ordering = u16_be(bytes, off + 6)?;
            axes.push(StatAxis {
                tag,
                axis_name_id,
                axis_ordering,
            });
        }

        let mut axis_values = Vec::with_capacity(axis_value_count);
        if axis_values_offset != 0 && axis_value_count > 0 {
            let off_table_end = axis_values_offset + axis_value_count * 2;
            if bytes.len() < off_table_end {
                return None;
            }
            for i in 0..axis_value_count {
                let off_off = axis_values_offset + i * 2;
                let rec_off = axis_values_offset + u16_be(bytes, off_off)? as usize;
                if rec_off + 8 > bytes.len() {
                    return None;
                }
                let format = u16_be(bytes, rec_off)?;
                match format {
                    1 => {
                        if rec_off + 12 > bytes.len() {
                            return None;
                        }
                        let axis_index = u16_be(bytes, rec_off + 2)?;
                        let flags = u16_be(bytes, rec_off + 4)?;
                        let value_name_id = u16_be(bytes, rec_off + 6)?;
                        let value = fixed_to_f32(i32_be(bytes, rec_off + 8)?);
                        axis_values.push(StatAxisValue::Single {
                            axis_index,
                            flags,
                            value_name_id,
                            value,
                        });
                    }
                    2 => {
                        if rec_off + 20 > bytes.len() {
                            return None;
                        }
                        let axis_index = u16_be(bytes, rec_off + 2)?;
                        let flags = u16_be(bytes, rec_off + 4)?;
                        let value_name_id = u16_be(bytes, rec_off + 6)?;
                        let nominal_value = fixed_to_f32(i32_be(bytes, rec_off + 8)?);
                        let range_min = fixed_to_f32(i32_be(bytes, rec_off + 12)?);
                        let range_max = fixed_to_f32(i32_be(bytes, rec_off + 16)?);
                        axis_values.push(StatAxisValue::Range {
                            axis_index,
                            flags,
                            value_name_id,
                            nominal_value,
                            range_min,
                            range_max,
                        });
                    }
                    3 => {
                        if rec_off + 16 > bytes.len() {
                            return None;
                        }
                        let axis_index = u16_be(bytes, rec_off + 2)?;
                        let flags = u16_be(bytes, rec_off + 4)?;
                        let value_name_id = u16_be(bytes, rec_off + 6)?;
                        let value = fixed_to_f32(i32_be(bytes, rec_off + 8)?);
                        let linked_value = fixed_to_f32(i32_be(bytes, rec_off + 12)?);
                        axis_values.push(StatAxisValue::Linked {
                            axis_index,
                            flags,
                            value_name_id,
                            value,
                            linked_value,
                        });
                    }
                    4 => {
                        let axis_count = u16_be(bytes, rec_off + 2)? as usize;
                        let flags = u16_be(bytes, rec_off + 4)?;
                        let value_name_id = u16_be(bytes, rec_off + 6)?;
                        let per_axis_off = rec_off + 8;
                        let need = per_axis_off + axis_count * 6;
                        if need > bytes.len() {
                            return None;
                        }
                        let mut per_axis = Vec::with_capacity(axis_count);
                        for k in 0..axis_count {
                            let p = per_axis_off + k * 6;
                            let ai = u16_be(bytes, p)?;
                            let v = fixed_to_f32(i32_be(bytes, p + 2)?);
                            per_axis.push((ai, v));
                        }
                        axis_values.push(StatAxisValue::Combined {
                            flags,
                            value_name_id,
                            per_axis,
                        });
                    }
                    _ => {
                        // Unknown format — skip silently. The spec
                        // reserves room for future formats and tells
                        // implementations to ignore unrecognised ones.
                    }
                }
            }
        }
        Some(Self {
            axes,
            axis_values,
            elided_fallback_name_id,
        })
    }

    pub fn axes(&self) -> &[StatAxis] {
        &self.axes
    }

    pub fn axis_values(&self) -> &[StatAxisValue] {
        &self.axis_values
    }

    pub fn elided_fallback_name_id(&self) -> u16 {
        self.elided_fallback_name_id
    }
}

#[inline]
fn fixed_to_f32(raw: i32) -> f32 {
    raw as f32 / 65536.0
}

// ---------------------------------------------------------------------
// `name` table snapshot — `name_id → String` resolver.
// ---------------------------------------------------------------------

/// Parsed `name` table snapshot. Walks every record, decodes UTF-16-BE
/// (Windows + Unicode platforms) or 7-bit-ASCII-of-Mac-Roman
/// (Mac platform 1, encoding 0) on the fly, and exposes a single
/// `find(name_id)` accessor that returns the highest-ranked match.
#[derive(Clone)]
pub struct NameTableSnapshot {
    /// `(rank, name_id, decoded_string)` — keep them all so multiple
    /// callers can pick a preferred record.
    records: Vec<(i32, u16, String)>,
}

impl Debug for NameTableSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NameTableSnapshot")
            .field("records", &self.records.len())
            .finish()
    }
}

impl NameTableSnapshot {
    /// Parse the raw bytes of a `name` table.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 6 {
            return None;
        }
        let format = u16_be(bytes, 0)?;
        if format > 1 {
            return None;
        }
        let count = u16_be(bytes, 2)? as usize;
        let string_offset = u16_be(bytes, 4)? as usize;
        let need = 6 + count * 12;
        if bytes.len() < need || string_offset > bytes.len() {
            return None;
        }
        let mut records = Vec::with_capacity(count);
        for i in 0..count {
            let off = 6 + i * 12;
            let platform = u16_be(bytes, off)?;
            let encoding = u16_be(bytes, off + 2)?;
            let language = u16_be(bytes, off + 4)?;
            let name_id = u16_be(bytes, off + 6)?;
            let length = u16_be(bytes, off + 8)? as usize;
            let str_off = u16_be(bytes, off + 10)? as usize;
            let start = string_offset + str_off;
            let end = match start.checked_add(length) {
                Some(e) if e <= bytes.len() => e,
                _ => continue,
            };
            let raw = &bytes[start..end];
            let rank = rank_record(platform, encoding, language);
            let decoded = match decode_name(platform, encoding, raw) {
                Some(s) => s,
                None => continue,
            };
            records.push((rank, name_id, decoded));
        }
        Some(Self { records })
    }

    /// Find the highest-ranked decoded string for `name_id`, or
    /// fall back to any record if no preferred one exists. Returns
    /// `None` only if no record carries that id.
    pub fn find(&self, name_id: u16) -> Option<&str> {
        let mut best: Option<(i32, &str)> = None;
        for (rank, nid, s) in &self.records {
            if *nid != name_id {
                continue;
            }
            match best {
                Some((br, _)) if br >= *rank => {}
                _ => best = Some((*rank, s.as_str())),
            }
        }
        best.map(|(_, s)| s)
    }
}

fn rank_record(platform: u16, encoding: u16, language: u16) -> i32 {
    match (platform, encoding, language) {
        (3, 1, 0x0409) => 100, // Windows Unicode English (US)
        (3, 1, l) if l & 0xFF == 9 => 90,
        (3, 1, _) => 80,
        (3, 10, _) => 75, // Windows UCS-4
        (1, 0, 0) => 70,
        (0, _, _) => 60,
        _ => 10,
    }
}

fn decode_name(platform: u16, encoding: u16, raw: &[u8]) -> Option<String> {
    match (platform, encoding) {
        (0, _) | (3, 1) | (3, 10) => {
            if raw.len() % 2 != 0 {
                return None;
            }
            let mut s = String::with_capacity(raw.len() / 2);
            let mut i = 0;
            while i + 1 < raw.len() {
                let u = u16::from_be_bytes([raw[i], raw[i + 1]]);
                i += 2;
                if (0xD800..=0xDBFF).contains(&u) {
                    if i + 1 >= raw.len() {
                        return None;
                    }
                    let lo = u16::from_be_bytes([raw[i], raw[i + 1]]);
                    if !(0xDC00..=0xDFFF).contains(&lo) {
                        return None;
                    }
                    i += 2;
                    let cp = 0x10000 + (((u - 0xD800) as u32) << 10) + (lo - 0xDC00) as u32;
                    s.push(char::from_u32(cp)?);
                } else {
                    s.push(char::from_u32(u as u32)?);
                }
            }
            Some(s)
        }
        (1, 0) => Some(
            raw.iter()
                .map(|&b| if b < 0x80 { b as char } else { '?' })
                .collect(),
        ),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// CFF2 — minimal INDEX walker + Top DICT reader for axis count + glyph
// count.
// ---------------------------------------------------------------------

/// Parsed CFF2 table — round-1 surface only. We expose the axis count
/// the variation store was built against (i.e. the matching `fvar`
/// axis count) and the glyph count, so callers can confirm the font's
/// CFF2 structure parses + matches `fvar`.
///
/// Full Type 2 v3 charstring evaluation under variations (op `blend`)
/// is deferred to a follow-up round.
#[derive(Debug, Clone)]
pub struct Cff2Table {
    /// Glyph count taken from the CharStrings INDEX.
    pub glyph_count: u32,
    /// Axis count from the variation store (matches `fvar`).
    pub axis_count: u16,
    /// `true` if the table parsed end-to-end and a non-empty
    /// CharStrings INDEX was found.
    pub has_charstrings: bool,
}

impl Cff2Table {
    /// Parse the raw bytes of a CFF2 table. Returns `None` if the
    /// header or any of the top-level structures fail to walk.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 5 {
            return None;
        }
        let major = bytes[0];
        let _minor = bytes[1];
        let header_size = bytes[2] as usize;
        let top_dict_length = u16_be(bytes, 3)? as usize;
        if major != 2 {
            return None;
        }
        if header_size < 5 || header_size + top_dict_length > bytes.len() {
            return None;
        }
        // Top DICT lives inline (NOT inside an INDEX) for CFF2.
        let top_dict = &bytes[header_size..header_size + top_dict_length];
        let cursor = header_size + top_dict_length;

        // Optional Global Subrs INDEX. We walk it to validate the
        // file structure even though we don't reuse `cursor` past
        // here — the remaining top-level data (CharStrings INDEX,
        // FDArray INDEX, FDSelect, VariationStore) is reachable via
        // absolute Top DICT operator offsets, not by sequential
        // cursor advance.
        let _global_subrs_end = cff2_index_end(bytes, cursor)?;

        // Walk Top DICT — operators 17 (CharStrings) and 24
        // (vstore) carry the table-relative offsets we need.
        let (charstrings_off, vstore_off) = parse_cff2_top_dict(top_dict)?;

        let glyph_count = if let Some(cs_off) = charstrings_off {
            cff2_index_count(bytes, cs_off as usize).unwrap_or(0)
        } else {
            0
        };

        // VariationStore in CFF2 starts with a u16 length, then the
        // ItemVariationStore proper.
        let axis_count = match vstore_off {
            Some(v) => {
                let v = v as usize;
                if v + 2 > bytes.len() {
                    0
                } else {
                    let _len = u16_be(bytes, v)?;
                    let store = ItemVariationStore::parse(bytes, v + 2)?;
                    store.axis_count()
                }
            }
            None => 0,
        };

        Some(Self {
            glyph_count,
            axis_count,
            has_charstrings: glyph_count > 0,
        })
    }
}

/// Walk a CFF2 INDEX and return the byte-offset just past its end. The
/// CFF2 INDEX header is `count: u32; offSize: u8; offsets[count+1]`.
fn cff2_index_end(bytes: &[u8], offset: usize) -> Option<usize> {
    if offset + 4 > bytes.len() {
        return None;
    }
    let count = u32_be(bytes, offset)?;
    if count == 0 {
        return Some(offset + 4);
    }
    if offset + 5 > bytes.len() {
        return None;
    }
    let off_size = bytes[offset + 4] as usize;
    if !(1..=4).contains(&off_size) {
        return None;
    }
    let off_array_off = offset + 5;
    let off_array_end = off_array_off + off_size * (count as usize + 1);
    if off_array_end > bytes.len() {
        return None;
    }
    // The last offset in the array is one-past-end of the data area
    // (CFF offsets are 1-based; subtract 1 to get the relative byte
    // offset into the data area).
    let last_off_off = off_array_off + off_size * count as usize;
    let mut last = 0u32;
    for i in 0..off_size {
        last = (last << 8) | bytes[last_off_off + i] as u32;
    }
    let data_area_end = off_array_end + (last as usize).saturating_sub(1);
    if data_area_end > bytes.len() {
        return None;
    }
    Some(data_area_end)
}

/// Read an INDEX's `count` field at `offset`.
fn cff2_index_count(bytes: &[u8], offset: usize) -> Option<u32> {
    if offset + 4 > bytes.len() {
        return None;
    }
    u32_be(bytes, offset)
}

/// Parse a CFF2 Top DICT and return `(charstrings_offset, vstore_offset)`.
/// Only operators 17 and 24 are recognised; everything else is skipped.
fn parse_cff2_top_dict(dict: &[u8]) -> Option<(Option<u32>, Option<u32>)> {
    let mut cursor = 0usize;
    let mut operands: Vec<i32> = Vec::with_capacity(8);
    let mut charstrings: Option<u32> = None;
    let mut vstore: Option<u32> = None;
    while cursor < dict.len() {
        let b0 = dict[cursor];
        cursor += 1;
        if b0 <= 21 {
            // Operator (single-byte 0..=21, with 12 being a 2-byte
            // escape lead-in — we don't care about any escape ops here
            // so just skip the trailing byte).
            let op = if b0 == 12 {
                if cursor >= dict.len() {
                    return None;
                }
                let b1 = dict[cursor];
                cursor += 1;
                0x0C00 | b1 as u16
            } else {
                b0 as u16
            };
            let last = operands.last().copied();
            match op {
                17 => {
                    if let Some(v) = last {
                        if v >= 0 {
                            charstrings = Some(v as u32);
                        }
                    }
                }
                24 => {
                    if let Some(v) = last {
                        if v >= 0 {
                            vstore = Some(v as u32);
                        }
                    }
                }
                _ => {}
            }
            operands.clear();
        } else {
            // Operand.
            let v = match b0 {
                28 => {
                    if cursor + 2 > dict.len() {
                        return None;
                    }
                    let v = i16::from_be_bytes([dict[cursor], dict[cursor + 1]]) as i32;
                    cursor += 2;
                    v
                }
                29 => {
                    if cursor + 4 > dict.len() {
                        return None;
                    }
                    let v = i32::from_be_bytes([
                        dict[cursor],
                        dict[cursor + 1],
                        dict[cursor + 2],
                        dict[cursor + 3],
                    ]);
                    cursor += 4;
                    v
                }
                30 => {
                    // BCD real — skip until the terminator nibble (0xF).
                    let mut done = false;
                    while !done {
                        if cursor >= dict.len() {
                            return None;
                        }
                        let b = dict[cursor];
                        cursor += 1;
                        if (b & 0x0F) == 0x0F || (b & 0xF0) == 0xF0 {
                            done = true;
                        }
                    }
                    0
                }
                32..=246 => b0 as i32 - 139,
                247..=250 => {
                    if cursor >= dict.len() {
                        return None;
                    }
                    let b1 = dict[cursor] as i32;
                    cursor += 1;
                    (b0 as i32 - 247) * 256 + b1 + 108
                }
                251..=254 => {
                    if cursor >= dict.len() {
                        return None;
                    }
                    let b1 = dict[cursor] as i32;
                    cursor += 1;
                    -((b0 as i32 - 251) * 256) - b1 - 108
                }
                _ => return None,
            };
            operands.push(v);
        }
    }
    Some((charstrings, vstore))
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_snapshot_decodes_utf16_be() {
        // Build a minimal name table with a single Windows English
        // record carrying name id 256 → "Inter".
        let s = "Inter";
        let utf16: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        let length = utf16.len() as u16;
        let header_size = 6 + 12;
        let mut out = vec![0u8; header_size];
        out[2..4].copy_from_slice(&1u16.to_be_bytes()); // count
        out[4..6].copy_from_slice(&(header_size as u16).to_be_bytes()); // stringOffset
        out[6..8].copy_from_slice(&3u16.to_be_bytes()); // platform
        out[8..10].copy_from_slice(&1u16.to_be_bytes()); // encoding
        out[10..12].copy_from_slice(&0x0409u16.to_be_bytes()); // lang
        out[12..14].copy_from_slice(&256u16.to_be_bytes()); // name id
        out[14..16].copy_from_slice(&length.to_be_bytes()); // length
        out[16..18].copy_from_slice(&0u16.to_be_bytes()); // offset
        out.extend_from_slice(&utf16);
        let n = NameTableSnapshot::parse(&out).expect("parse");
        assert_eq!(n.find(256), Some("Inter"));
        assert_eq!(n.find(257), None);
    }

    #[test]
    fn item_variation_store_resolves_zero_for_default_coords() {
        // Synthetic IVS with one region (axis 0: [-1, +1, +1]) and one
        // delta-set with one byte delta == 10. At coord = 0.0 the
        // scalar is 0 → resolved delta is 0.
        //
        // Layout (sizes / offsets in parens):
        //   IVS header (12): format(2) regionListOffset(4) ivdCount(2) ivdOffset[0](4)
        //   VRL (10) at offset 12: axisCount(2) regionCount(2) region[0]: 6 bytes
        //   IVD at offset 22: itemCount(2) wordDeltaCount(2) regionIndexCount(2)
        //                     regionIndexes[0](2) delta(1) = 9 bytes
        //   total: 31 bytes.
        let region_list_off = 12u32;
        let ivd_off = 22u32;
        let mut b = Vec::new();
        b.extend_from_slice(&1u16.to_be_bytes()); // format = 1
        b.extend_from_slice(&region_list_off.to_be_bytes());
        b.extend_from_slice(&1u16.to_be_bytes()); // ivd count
        b.extend_from_slice(&ivd_off.to_be_bytes());
        // VariationRegionList @ region_list_off.
        debug_assert_eq!(b.len() as u32, region_list_off);
        b.extend_from_slice(&1u16.to_be_bytes()); // axisCount
        b.extend_from_slice(&1u16.to_be_bytes()); // regionCount
                                                  // region[0]: start=-1 peak=1 end=1
        b.extend_from_slice(&(-16384i16).to_be_bytes());
        b.extend_from_slice(&(16384i16).to_be_bytes());
        b.extend_from_slice(&(16384i16).to_be_bytes());
        // ItemVariationData @ ivd_off.
        debug_assert_eq!(b.len() as u32, ivd_off);
        b.extend_from_slice(&1u16.to_be_bytes()); // itemCount
        b.extend_from_slice(&0u16.to_be_bytes()); // wordDeltaCount = 0 (all bytes)
        b.extend_from_slice(&1u16.to_be_bytes()); // regionIndexCount
        b.extend_from_slice(&0u16.to_be_bytes()); // regionIndexes[0] = 0
        b.push(10u8); // delta = 10 byte
        let s = ItemVariationStore::parse(&b, 0).expect("parse");
        // At default coord (0.0): per spec, "if axis position is 0,
        // contribution is 0" → resolved delta is 0 regardless of peak.
        assert!((s.resolve_delta(0, 0, &[0.0]) - 0.0).abs() < 1e-6);
        // At peak (1.0): full delta = 10.
        assert!((s.resolve_delta(0, 0, &[1.0]) - 10.0).abs() < 1e-6);
        // Region [start=-1, peak=1, end=1] with coord 0.5 ramps from
        // start (-1) to peak (+1) — the (c - start) / (peak - start)
        // branch — so scalar = 1.5/2 = 0.75. delta = 0.75 × 10 = 7.5.
        assert!((s.resolve_delta(0, 0, &[0.5]) - 7.5).abs() < 1e-6);
    }

    #[test]
    fn delta_set_index_map_resolves_short_form() {
        // Format 0, entrySize=1, innerBits=4 → outer in high 4 bits,
        // inner in low 4. mapCount=3, entries: 0x10, 0x21, 0x32.
        let raw = [
            0u8, // format
            0x30, // entryFormat: entrySize-1 = 0 → entrySize=1; innerBits-1 = 0 → innerBits=1.
                 // Hmm, with innerBits=1 the mask is 1 → outer = entry>>1, inner = entry & 1.
        ];
        // Build correctly: entrySize=1, innerBits=4 → entryFormat = (0<<4)|3 = 0x03.
        let mut b = vec![
            0u8,  // format=0
            0x03, // entrySize=1, innerBits=4
            0u8, 3, // mapCount=3
            0x10, 0x21, 0x32,
        ];
        let _ = raw;
        let m = DeltaSetIndexMap::parse(&b, 0).expect("parse");
        assert_eq!(m.resolve(0), (1, 0));
        assert_eq!(m.resolve(1), (2, 1));
        assert_eq!(m.resolve(2), (3, 2));
        // Out-of-range key uses the last entry.
        assert_eq!(m.resolve(99), (3, 2));
        // entryFormat=0x13 → entrySize=2, innerBits=4 → 16-bit BE entry.
        b = vec![
            0u8, 0x13, 0u8, 1, 0x00, 0x42, // outer=4, inner=2
        ];
        let m2 = DeltaSetIndexMap::parse(&b, 0).expect("parse");
        assert_eq!(m2.resolve(0), (4, 2));
    }

    #[test]
    fn cff2_index_walker_handles_empty_and_nonempty() {
        // Empty INDEX: count=0 → 4 bytes total.
        let empty = [0u8; 4];
        assert_eq!(cff2_index_end(&empty, 0), Some(4));
        // 1-entry INDEX: count=1, offSize=1, offsets=[1, 4] (data is
        // 3 bytes: 0xAA 0xBB 0xCC). Offset array length = (count+1)*offSize
        // = 2 bytes. Data area = last_offset (4) - 1 = 3 bytes.
        let mut b = Vec::new();
        b.extend_from_slice(&1u32.to_be_bytes()); // count
        b.push(1u8); // offSize
        b.push(1u8); // offsets[0]
        b.push(4u8); // offsets[1]
        b.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        assert_eq!(cff2_index_end(&b, 0), Some(b.len()));
    }

    #[test]
    fn stat_axis_value_format1_round_trip() {
        // Build a STAT v1.1 with 1 axis (wght), 1 axis-value (Single,
        // wght=400 → name_id 256).
        //
        // Layout:
        //   header (20): major(2) minor(2) dasize(2) dacount(2)
        //                daoffset(4) avcount(2) avoffset(4) elidedFB(2)
        //   design axes (8) at 20: tag(4) nameID(2) ordering(2)
        //   av offset table (2) at 28: u16 rel offset
        //   av record (12) at 30: format(2) axisIndex(2) flags(2)
        //                          nameID(2) value(4)
        //   total: 42 bytes.
        let header_size = 20usize;
        let design_axes_offset = header_size;
        let design_axis_size = 8usize;
        let axis_value_offset_table = design_axes_offset + design_axis_size;
        let av_record_offset = axis_value_offset_table + 2;
        let mut b = vec![0u8; av_record_offset + 12];
        b[0..2].copy_from_slice(&1u16.to_be_bytes()); // major
        b[2..4].copy_from_slice(&1u16.to_be_bytes()); // minor
        b[4..6].copy_from_slice(&8u16.to_be_bytes()); // designAxisSize
        b[6..8].copy_from_slice(&1u16.to_be_bytes()); // designAxisCount
        b[8..12].copy_from_slice(&(design_axes_offset as u32).to_be_bytes());
        b[12..14].copy_from_slice(&1u16.to_be_bytes()); // axisValueCount
        b[14..18].copy_from_slice(&(axis_value_offset_table as u32).to_be_bytes());
        b[18..20].copy_from_slice(&2u16.to_be_bytes()); // elidedFallback (=2)
        b[design_axes_offset..design_axes_offset + 4].copy_from_slice(b"wght");
        b[design_axes_offset + 4..design_axes_offset + 6].copy_from_slice(&256u16.to_be_bytes()); // axisNameID
                                                                                                  // axisOrdering 0 already.
                                                                                                  // Axis-value offset (relative to offset table base):
        let av_rel = (av_record_offset - axis_value_offset_table) as u16;
        b[axis_value_offset_table..axis_value_offset_table + 2]
            .copy_from_slice(&av_rel.to_be_bytes());
        // Format-1 record: format=1, axisIndex=0, flags=0,
        // valueNameID=257, value=400.0 (Fixed 16.16).
        let r = av_record_offset;
        b[r..r + 2].copy_from_slice(&1u16.to_be_bytes());
        b[r + 2..r + 4].copy_from_slice(&0u16.to_be_bytes());
        b[r + 4..r + 6].copy_from_slice(&0u16.to_be_bytes());
        b[r + 6..r + 8].copy_from_slice(&257u16.to_be_bytes());
        b[r + 8..r + 12].copy_from_slice(&((400i32) << 16).to_be_bytes());
        let _ = av_rel;
        let _ = av_record_offset;
        let s = StatTable::parse(&b).expect("parse");
        assert_eq!(s.axes().len(), 1);
        assert_eq!(&s.axes()[0].tag, b"wght");
        assert_eq!(s.axes()[0].axis_name_id, 256);
        assert_eq!(s.elided_fallback_name_id(), 2);
        match &s.axis_values()[0] {
            StatAxisValue::Single {
                axis_index,
                value_name_id,
                value,
                ..
            } => {
                assert_eq!(*axis_index, 0);
                assert_eq!(*value_name_id, 257);
                assert!((value - 400.0).abs() < 1e-3);
            }
            other => panic!("wrong format: {other:?}"),
        }
    }
}
