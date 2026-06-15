//! Unicode Character Database property lookups backing the UAX #9
//! engine — `Bidi_Class` ([`class_lookup`]), `Bidi_Mirroring_Glyph`
//! ([`mirror_lookup`]), and `Bidi_Paired_Bracket` /
//! `Bidi_Paired_Bracket_Type` ([`bracket_lookup`]).
//!
//! ## Provenance
//!
//! `Bidi_Class` and `Bidi_Mirroring_Glyph` are delegated to the
//! Karpelès Lab `intl` crate (a pure-Rust internationalization
//! library), which compiles the UCD property tables into const-fn
//! lookups. We map its `unicode::bidi::BidiClass` enum onto our own
//! [`BidiClass`] and its `unicode::bidi_mirror` onto [`mirror_lookup`].
//! This replaces the per-call `DerivedBidiClass.txt` /
//! `BidiMirroring.txt` runtime parsers that previously lived here.
//!
//! `Bidi_Paired_Bracket` is not exposed by `intl`, so the
//! `BidiBrackets.txt` snapshot staged under `docs/text/unicode-bidi/`
//! is still vendored next to this file and parsed once into a sorted
//! table behind a [`OnceLock`].
//!
//! ## `BidiBrackets.txt` format (UAX #44 conventions)
//!
//! `<cp>; <paired-cp>; o|c # name` lines give the normative
//! Bidi_Paired_Bracket (the matching bracket) and
//! Bidi_Paired_Bracket_Type (`o` = Open per BD14, `c` = Close per
//! BD15) of every paired-bracket character (consumed by BD16 / N0).

use std::sync::OnceLock;

use intl::unicode::bidi::BidiClass as IntlBidiClass;

use super::{BidiClass, BracketKind};

const BIDI_BRACKETS: &str = include_str!("BidiBrackets.txt");

/// Map `intl`'s `Bidi_Class` enum onto this crate's [`BidiClass`].
/// Both enumerate the 23 UAX #9 §3.2 Table 4 classes; this is a pure
/// rename with no behavioural change.
fn from_intl(c: IntlBidiClass) -> BidiClass {
    match c {
        IntlBidiClass::L => BidiClass::L,
        IntlBidiClass::R => BidiClass::R,
        IntlBidiClass::AL => BidiClass::AL,
        IntlBidiClass::EN => BidiClass::EN,
        IntlBidiClass::ES => BidiClass::ES,
        IntlBidiClass::ET => BidiClass::ET,
        IntlBidiClass::AN => BidiClass::AN,
        IntlBidiClass::CS => BidiClass::CS,
        IntlBidiClass::NSM => BidiClass::NSM,
        IntlBidiClass::BN => BidiClass::BN,
        IntlBidiClass::B => BidiClass::B,
        IntlBidiClass::S => BidiClass::S,
        IntlBidiClass::WS => BidiClass::WS,
        IntlBidiClass::ON => BidiClass::ON,
        IntlBidiClass::LRE => BidiClass::LRE,
        IntlBidiClass::LRO => BidiClass::LRO,
        IntlBidiClass::RLE => BidiClass::RLE,
        IntlBidiClass::RLO => BidiClass::RLO,
        IntlBidiClass::PDF => BidiClass::PDF,
        IntlBidiClass::LRI => BidiClass::LRI,
        IntlBidiClass::RLI => BidiClass::RLI,
        IntlBidiClass::FSI => BidiClass::FSI,
        IntlBidiClass::PDI => BidiClass::PDI,
    }
}

/// UAX #9 §3.2 `Bidi_Class` `@missing` block defaults for **unassigned**
/// code points, as published in the Unicode `DerivedBidiClass.txt`
/// header. The algorithm assigns strong types to unassigned code
/// points "in blocks reserved for right-to-left scripts" (`R` / `AL`)
/// and `ET` to the Currency Symbols block — "an explicit exception to
/// the general Unicode conformance requirements with respect to
/// unassigned characters." Every code point outside these ranges
/// defaults to `L` (the global `0000..10FFFF; Left_To_Right` line).
///
/// `intl`'s `bidi_class` returns the **assigned** class for assigned
/// code points but `L` for unassigned ones, so this overlay is applied
/// only when `intl` reports `L` and `cp` falls inside one of these
/// blocks — restoring the §3.2 default the previous
/// `DerivedBidiClass.txt` parser produced. (No assigned code point in
/// these RTL / Currency blocks has `Bidi_Class = L`, so overlaying on
/// an `L` result never overrides a real assignment.)
const MISSING_BLOCKS: &[(u32, u32, BidiClass)] = &[
    (0x0590, 0x05FF, BidiClass::R),
    (0x0600, 0x07BF, BidiClass::AL),
    (0x07C0, 0x085F, BidiClass::R),
    (0x0860, 0x08FF, BidiClass::AL),
    (0x20A0, 0x20CF, BidiClass::ET),
    (0xFB1D, 0xFB4F, BidiClass::R),
    (0xFB50, 0xFDCF, BidiClass::AL),
    (0xFDF0, 0xFDFF, BidiClass::AL),
    (0xFE70, 0xFEFF, BidiClass::AL),
    (0x1_0800, 0x1_0CFF, BidiClass::R),
    (0x1_0D00, 0x1_0D3F, BidiClass::AL),
    (0x1_0D40, 0x1_0EBF, BidiClass::R),
    (0x1_0EC0, 0x1_0EFF, BidiClass::AL),
    (0x1_0F00, 0x1_0F2F, BidiClass::R),
    (0x1_0F30, 0x1_0F6F, BidiClass::AL),
    (0x1_0F70, 0x1_0FFF, BidiClass::R),
    (0x1_E800, 0x1_EC6F, BidiClass::R),
    (0x1_EC70, 0x1_ECBF, BidiClass::AL),
    (0x1_ECC0, 0x1_ECFF, BidiClass::R),
    (0x1_ED00, 0x1_ED4F, BidiClass::AL),
    (0x1_ED50, 0x1_EDFF, BidiClass::R),
    (0x1_EE00, 0x1_EEFF, BidiClass::AL),
    (0x1_EF00, 0x1_EFFF, BidiClass::R),
];

/// `Bidi_Class` of `cp`. Assigned code points are resolved by `intl`'s
/// compiled UCD tables; unassigned code points get the UAX #9 §3.2
/// `@missing` block default ([`MISSING_BLOCKS`]) when they fall in a
/// right-to-left script block or the Currency Symbols block, and `L`
/// otherwise — matching the previous `DerivedBidiClass.txt` parser.
pub(super) fn class_lookup(cp: u32) -> BidiClass {
    let class = from_intl(intl::unicode::bidi::bidi_class_u32(cp));
    if class == BidiClass::L {
        for &(lo, hi, default) in MISSING_BLOCKS {
            if cp >= lo && cp <= hi {
                return default;
            }
        }
    }
    class
}

/// `Bidi_Mirroring_Glyph` of `c` via `intl`'s compiled UCD tables —
/// the acceptable mirror-pair character, or `None` when the character
/// has no mirror pair (consumed by rule L4).
pub(super) fn mirror_lookup(c: char) -> Option<char> {
    intl::unicode::bidi_mirror(c)
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
    fn class_lookup_matches_known_assignments() {
        // Spot-check representative code points across the UAX #9
        // §3.2 Table 4 classes — these assignments are stable across
        // Unicode versions, so they pin the `intl` delegation.
        assert_eq!(class_lookup('A' as u32), BidiClass::L);
        assert_eq!(class_lookup('\u{05D0}' as u32), BidiClass::R); // Hebrew alef
        assert_eq!(class_lookup('\u{0627}' as u32), BidiClass::AL); // Arabic alef
        assert_eq!(class_lookup('5' as u32), BidiClass::EN);
        assert_eq!(class_lookup('\u{0660}' as u32), BidiClass::AN); // Arabic-Indic 0
        assert_eq!(class_lookup('\u{0300}' as u32), BidiClass::NSM); // combining grave
        assert_eq!(class_lookup(' ' as u32), BidiClass::WS);
        assert_eq!(class_lookup('(' as u32), BidiClass::ON);
        assert_eq!(class_lookup('\u{202A}' as u32), BidiClass::LRE);
        assert_eq!(class_lookup('\u{2067}' as u32), BidiClass::RLI);
        assert_eq!(class_lookup('\u{2069}' as u32), BidiClass::PDI);
        // Unassigned code point inside the Hebrew block defaults to R
        // per the §3.2 @missing rule.
        assert_eq!(class_lookup(0x05EB), BidiClass::R);
    }

    #[test]
    fn mirror_lookup_is_an_involution_for_brackets() {
        // Every paired bracket has a mirror, and mirroring is an
        // involution; verify via the bracket table (which `intl` does
        // not supply) so the two property sources stay consistent.
        for &(cp, paired, _) in bracket_table() {
            let c = char::from_u32(cp).expect("valid scalar");
            let p = char::from_u32(paired).expect("valid scalar");
            assert_eq!(mirror_lookup(c), Some(p), "U+{cp:04X} bmg");
            assert_eq!(mirror_lookup(p), Some(c), "U+{cp:04X} not an involution");
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
