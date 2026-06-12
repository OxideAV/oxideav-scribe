//! Unicode Character Database data tables backing the UAX #9
//! property lookups — `Bidi_Class` ([`class_lookup`], from
//! `DerivedBidiClass.txt`), `Bidi_Mirroring_Glyph` ([`mirror_lookup`],
//! from `BidiMirroring.txt`), and `Bidi_Paired_Bracket` /
//! `Bidi_Paired_Bracket_Type` ([`bracket_lookup`], from
//! `BidiBrackets.txt`).
//!
//! The three data files are the Unicode **16.0** UCD snapshots staged
//! under `docs/text/unicode-bidi/` (the same Unicode version as the
//! pinned UAX #9 Revision 50 spec snapshot), vendored verbatim into
//! this directory with their copyright / terms-of-use headers intact.
//! Each is embedded via `include_str!` and parsed exactly once, on
//! first lookup, into a sorted range table behind a [`OnceLock`];
//! every subsequent lookup is a binary search.
//!
//! ## File formats (UAX #44 conventions)
//!
//! - `DerivedBidiClass.txt` — `<cp>[..<cp>] ; <short-class> # comment`
//!   lines list the Bidi_Class of every **assigned** code point.
//!   `# @missing: <cp>..<cp>; <Long_Class_Name>` header lines give
//!   the defaults for **unassigned** code points: `Right_To_Left` /
//!   `Arabic_Letter` for the blocks reserved for right-to-left
//!   scripts, `European_Terminator` for the Currency Symbols block,
//!   and the global `0000..10FFFF; Left_To_Right` fallback for
//!   everything else ("All code points not explicitly listed for
//!   Bidi_Class have the value Left_To_Right (L)").
//! - `BidiMirroring.txt` — `<cp>; <mirror-cp> # name` lines map each
//!   `Bidi_Mirrored=Yes` character with an acceptable mirror pair to
//!   that pair (UAX #9 §7 *Mirroring*; consumed by rule L4). The
//!   mapping is an involution: both directions are listed.
//! - `BidiBrackets.txt` — `<cp>; <paired-cp>; o|c # name` lines give
//!   the normative Bidi_Paired_Bracket (the matching bracket) and
//!   Bidi_Paired_Bracket_Type (`o` = Open per BD14, `c` = Close per
//!   BD15) of every paired-bracket character (consumed by BD16 / N0).

use std::sync::OnceLock;

use super::{BidiClass, BracketKind};

const DERIVED_BIDI_CLASS: &str = include_str!("DerivedBidiClass.txt");
const BIDI_MIRRORING: &str = include_str!("BidiMirroring.txt");
const BIDI_BRACKETS: &str = include_str!("BidiBrackets.txt");

/// Parsed `DerivedBidiClass.txt`: explicit per-code-point ranges plus
/// the `@missing` defaults for unassigned code points.
struct ClassTable {
    /// `(first, last, class)` for every data line, sorted by `first`,
    /// pairwise disjoint.
    explicit: Vec<(u32, u32, BidiClass)>,
    /// `(first, last, class)` for every block-specific `@missing`
    /// line (the global `0000..10FFFF; Left_To_Right` line is folded
    /// into the final `L` fallback instead), sorted by `first`,
    /// pairwise disjoint.
    missing: Vec<(u32, u32, BidiClass)>,
}

/// Map a short Bidi_Class alias (data-line field) to [`BidiClass`].
fn class_from_short(s: &str) -> Option<BidiClass> {
    Some(match s {
        "L" => BidiClass::L,
        "R" => BidiClass::R,
        "AL" => BidiClass::AL,
        "EN" => BidiClass::EN,
        "ES" => BidiClass::ES,
        "ET" => BidiClass::ET,
        "AN" => BidiClass::AN,
        "CS" => BidiClass::CS,
        "NSM" => BidiClass::NSM,
        "BN" => BidiClass::BN,
        "B" => BidiClass::B,
        "S" => BidiClass::S,
        "WS" => BidiClass::WS,
        "ON" => BidiClass::ON,
        "LRE" => BidiClass::LRE,
        "LRO" => BidiClass::LRO,
        "RLE" => BidiClass::RLE,
        "RLO" => BidiClass::RLO,
        "PDF" => BidiClass::PDF,
        "LRI" => BidiClass::LRI,
        "RLI" => BidiClass::RLI,
        "FSI" => BidiClass::FSI,
        "PDI" => BidiClass::PDI,
        _ => return None,
    })
}

/// Map a long Bidi_Class property-value name (`@missing` field) to
/// [`BidiClass`]. Only the values that actually occur in the Unicode
/// 16.0 `@missing` lines are mapped.
fn class_from_long(s: &str) -> Option<BidiClass> {
    Some(match s {
        "Left_To_Right" => BidiClass::L,
        "Right_To_Left" => BidiClass::R,
        "Arabic_Letter" => BidiClass::AL,
        "European_Terminator" => BidiClass::ET,
        "Boundary_Neutral" => BidiClass::BN,
        _ => return None,
    })
}

/// Parse a UAX #44 `<cp>` or `<cp>..<cp>` range field.
fn parse_range(field: &str) -> Option<(u32, u32)> {
    let field = field.trim();
    if let Some((lo, hi)) = field.split_once("..") {
        let lo = u32::from_str_radix(lo.trim(), 16).ok()?;
        let hi = u32::from_str_radix(hi.trim(), 16).ok()?;
        Some((lo, hi))
    } else {
        let cp = u32::from_str_radix(field, 16).ok()?;
        Some((cp, cp))
    }
}

fn class_table() -> &'static ClassTable {
    static TABLE: OnceLock<ClassTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut explicit = Vec::with_capacity(2304);
        let mut missing = Vec::new();
        for line in DERIVED_BIDI_CLASS.lines() {
            // `@missing` defaults live inside comment lines:
            //   # @missing: 0590..05FF; Right_To_Left
            if let Some(rest) = line.trim_start().strip_prefix("# @missing:") {
                let mut fields = rest.split(';');
                let range = fields.next().and_then(parse_range);
                let class = fields.next().and_then(|f| class_from_long(f.trim()));
                let (Some((lo, hi)), Some(class)) = (range, class) else {
                    panic!("DerivedBidiClass.txt: malformed @missing line: {line:?}");
                };
                // The global default ("All code points not explicitly
                // listed for Bidi_Class have the value Left_To_Right")
                // is the final fallback in `class_lookup`, not a
                // searched range.
                if (lo, hi) == (0, 0x0010_FFFF) {
                    assert_eq!(class, BidiClass::L, "global @missing default must be L");
                    continue;
                }
                missing.push((lo, hi, class));
                continue;
            }
            // Ordinary data line: strip the trailing comment, skip
            // blanks.
            let data = line.split('#').next().unwrap_or("").trim();
            if data.is_empty() {
                continue;
            }
            let mut fields = data.split(';');
            let range = fields.next().and_then(parse_range);
            let class = fields.next().and_then(|f| class_from_short(f.trim()));
            let (Some((lo, hi)), Some(class)) = (range, class) else {
                panic!("DerivedBidiClass.txt: malformed data line: {line:?}");
            };
            explicit.push((lo, hi, class));
        }
        // The file groups data lines by class value, so the ranges
        // are only sorted within each group — sort globally for the
        // binary search. Ranges from different groups are disjoint
        // (each code point has exactly one Bidi_Class); the
        // module-level tests assert this invariant.
        explicit.sort_unstable_by_key(|&(lo, _, _)| lo);
        missing.sort_unstable_by_key(|&(lo, _, _)| lo);
        ClassTable { explicit, missing }
    })
}

/// Binary-search a sorted disjoint `(first, last, value)` range table.
fn range_search<T: Copy>(table: &[(u32, u32, T)], cp: u32) -> Option<T> {
    let idx = table.partition_point(|&(lo, _, _)| lo <= cp);
    let (lo, hi, value) = *table.get(idx.checked_sub(1)?)?;
    debug_assert!(lo <= cp);
    (cp <= hi).then_some(value)
}

/// `Bidi_Class` of `cp` per `DerivedBidiClass.txt` (Unicode 16.0):
/// explicit assignment if listed, the block-specific `@missing`
/// default for unassigned code points in right-to-left script blocks
/// / the Currency Symbols block, and `L` otherwise.
pub(super) fn class_lookup(cp: u32) -> BidiClass {
    let table = class_table();
    range_search(&table.explicit, cp)
        .or_else(|| range_search(&table.missing, cp))
        .unwrap_or(BidiClass::L)
}

fn mirror_table() -> &'static Vec<(u32, u32)> {
    static TABLE: OnceLock<Vec<(u32, u32)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut entries = Vec::with_capacity(428);
        for line in BIDI_MIRRORING.lines() {
            let data = line.split('#').next().unwrap_or("").trim();
            if data.is_empty() {
                continue;
            }
            let mut fields = data.split(';');
            let (Some(cp), Some(mirror)) = (
                fields
                    .next()
                    .and_then(|f| u32::from_str_radix(f.trim(), 16).ok()),
                fields
                    .next()
                    .and_then(|f| u32::from_str_radix(f.trim(), 16).ok()),
            ) else {
                panic!("BidiMirroring.txt: malformed data line: {line:?}");
            };
            entries.push((cp, mirror));
        }
        entries.sort_unstable_by_key(|&(cp, _)| cp);
        entries
    })
}

/// `Bidi_Mirroring_Glyph` of `c` per `BidiMirroring.txt` (Unicode
/// 16.0) — the acceptable mirror-pair character, or `None` when the
/// character has no mirror pair (including every `Bidi_Mirrored=No`
/// character, e.g. the U+FD3E / U+FD3F ornate parentheses).
pub(super) fn mirror_lookup(c: char) -> Option<char> {
    let table = mirror_table();
    let idx = table
        .binary_search_by_key(&(c as u32), |&(cp, _)| cp)
        .ok()?;
    char::from_u32(table[idx].1)
}

fn bracket_table() -> &'static Vec<(u32, u32, BracketKind)> {
    static TABLE: OnceLock<Vec<(u32, u32, BracketKind)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut entries = Vec::with_capacity(128);
        for line in BIDI_BRACKETS.lines() {
            let data = line.split('#').next().unwrap_or("").trim();
            if data.is_empty() {
                continue;
            }
            let mut fields = data.split(';');
            let cp = fields
                .next()
                .and_then(|f| u32::from_str_radix(f.trim(), 16).ok());
            let paired = fields
                .next()
                .and_then(|f| u32::from_str_radix(f.trim(), 16).ok());
            let kind = fields.next().map(str::trim);
            let (Some(cp), Some(paired), Some(kind)) = (cp, paired, kind) else {
                panic!("BidiBrackets.txt: malformed data line: {line:?}");
            };
            let kind = match kind {
                "o" => BracketKind::Open,
                "c" => BracketKind::Close,
                other => panic!("BidiBrackets.txt: unknown Bidi_Paired_Bracket_Type {other:?}"),
            };
            entries.push((cp, paired, kind));
        }
        entries.sort_unstable_by_key(|&(cp, _, _)| cp);
        entries
    })
}

/// `Bidi_Paired_Bracket` + `Bidi_Paired_Bracket_Type` of `c` per
/// `BidiBrackets.txt` (Unicode 16.0) — `Some((paired_char, kind))`
/// when `c` is an opening (BD14) or closing (BD15) paired bracket,
/// `None` otherwise.
pub(super) fn bracket_lookup(c: char) -> Option<(char, BracketKind)> {
    let table = bracket_table();
    let idx = table
        .binary_search_by_key(&(c as u32), |&(cp, _, _)| cp)
        .ok()?;
    let (_, paired, kind) = table[idx];
    Some((char::from_u32(paired)?, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_table_is_sorted_and_disjoint() {
        let table = class_table();
        // Unicode 16.0 DerivedBidiClass.txt carries 2289 data lines
        // and 24 @missing lines, of which one is the global L
        // default (folded into the fallback) and 23 are
        // block-specific.
        assert_eq!(table.explicit.len(), 2289);
        assert_eq!(table.missing.len(), 23);
        for pair in table.explicit.windows(2) {
            let (lo_a, hi_a, _) = pair[0];
            let (lo_b, _, _) = pair[1];
            assert!(lo_a <= hi_a, "inverted range at U+{lo_a:04X}");
            assert!(hi_a < lo_b, "overlap at U+{lo_b:04X}");
        }
        for pair in table.missing.windows(2) {
            let (lo_a, hi_a, _) = pair[0];
            let (lo_b, _, _) = pair[1];
            assert!(lo_a <= hi_a, "inverted @missing range at U+{lo_a:04X}");
            assert!(hi_a < lo_b, "@missing overlap at U+{lo_b:04X}");
        }
    }

    #[test]
    fn mirror_table_is_a_full_involution() {
        // Unicode 16.0 BidiMirroring.txt carries 428 data lines, and
        // every mapping is listed in both directions.
        let table = mirror_table();
        assert_eq!(table.len(), 428);
        for &(cp, mirror) in table {
            let c = char::from_u32(cp).expect("valid scalar");
            let m = char::from_u32(mirror).expect("valid scalar");
            assert_eq!(mirror_lookup(m), Some(c), "U+{cp:04X} not an involution");
            assert_eq!(mirror_lookup(c), Some(m));
        }
    }

    #[test]
    fn bracket_table_pairs_open_with_close() {
        // Unicode 16.0 BidiBrackets.txt carries 128 data lines = 64
        // open/close pairs; the paired character of an Open entry is
        // a Close entry pointing back, and vice versa.
        let table = bracket_table();
        assert_eq!(table.len(), 128);
        let opens = table
            .iter()
            .filter(|&&(_, _, k)| k == BracketKind::Open)
            .count();
        assert_eq!(opens, 64);
        for &(cp, paired, kind) in table {
            let c = char::from_u32(cp).expect("valid scalar");
            let p = char::from_u32(paired).expect("valid scalar");
            let (back, back_kind) = bracket_lookup(p).expect("paired entry present");
            assert_eq!(back, c, "U+{cp:04X} paired-bracket not symmetric");
            let expected_back_kind = match kind {
                BracketKind::Open => BracketKind::Close,
                BracketKind::Close => BracketKind::Open,
            };
            assert_eq!(back_kind, expected_back_kind);
        }
    }

    #[test]
    fn every_paired_bracket_is_on_and_mirrored() {
        // BidiBrackets.txt header: "two characters, A and B, form a
        // bracket pair if A has gc=Ps and B has gc=Pe, both have
        // bc=ON and Bidi_M=Y, and bmg of A is B."
        for &(cp, paired, _) in bracket_table() {
            let c = char::from_u32(cp).expect("valid scalar");
            let p = char::from_u32(paired).expect("valid scalar");
            assert_eq!(class_lookup(cp), BidiClass::ON, "U+{cp:04X} class");
            assert_eq!(mirror_lookup(c), Some(p), "U+{cp:04X} bmg");
        }
    }
}
