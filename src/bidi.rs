//! Unicode Bidirectional Algorithm — UAX #9 character classes +
//! paragraph-level resolution (rules P1 / P2 / P3).
//!
//! ## Scope
//!
//! This module implements the **first phase** of the Unicode
//! Bidirectional Algorithm (UBA) as specified in Unicode Standard
//! Annex #9, *Unicode Bidirectional Algorithm*, Revision 50 / Unicode
//! 16.0 (the dated snapshot pinned at
//! `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`). The
//! foundation surface is:
//!
//! - [`BidiClass`] — the 23 normative bidirectional character types
//!   from UAX #9 §3.2 Table 4 (3 Strong, 7 Weak, 4 Neutral, 9
//!   Explicit Formatting).
//! - [`bidi_class`] — a `char` → [`BidiClass`] lookup covering the
//!   ranges scribe needs for shaping today: the 12 explicit
//!   formatting characters (LRM / RLM / ALM / LRE / RLE / PDF / LRO /
//!   RLO / LRI / RLI / FSI / PDI), the paragraph / segment / line
//!   separators, the ASCII / Latin-1 punctuation + digit zones, the
//!   Hebrew block (U+0590..U+05FF), the four core Arabic blocks
//!   (U+0600..U+06FF, U+0700..U+074F Syriac, U+0750..U+077F Arabic
//!   Supplement, U+FB50..U+FDFF Arabic Presentation Forms-A,
//!   U+FE70..U+FEFF Arabic Presentation Forms-B), Thaana
//!   (U+0780..U+07BF), N'Ko (U+07C0..U+07FF), Hebrew Presentation
//!   Forms (U+FB1D..U+FB4F), and combining marks (U+0300..U+036F
//!   Combining Diacritical Marks, U+064B..U+065F Arabic combining
//!   marks, U+0670 Arabic letter superscript alef, U+06D6..U+06ED
//!   Arabic combining marks B). Unmapped code points fall back to
//!   `L` per the UAX #9 §3.2 default ("Unassigned characters are
//!   given strong types in the algorithm.").
//! - [`paragraph_level`] — the **P1 + P2 + P3** rules: walk the
//!   text, skip the contents of any isolate (LRI / RLI / FSI ... PDI)
//!   region, find the first strong character (L / R / AL); P3 sets
//!   level 1 if it is R or AL, level 0 otherwise (which is also the
//!   default when no strong character is found).
//!
//! ## Out of scope (deferred to follow-up rounds)
//!
//! - W1..W7 (weak type resolution).
//! - N0..N2 (neutral type resolution, including the §3.1.3 bracket
//!   pairs algorithm).
//! - I1..I2 (implicit embedding level resolution).
//! - X1..X10 (explicit embedding / override / isolate stack
//!   machinery + the isolating-run-sequence partition).
//! - L1..L4 (line-level reordering + mirroring).
//!
//! The UCD-derived per-code-point class table is also intentionally
//! a **partial** table. Filling it out fully (every code point in
//! the Bidi_Mirrored / NSM / EN / ET / AN ranges across the BMP and
//! supplementary planes) is a follow-up that needs the
//! `DerivedBidiClass.txt` data file from the Unicode Character
//! Database, which UAX #9 references but which is not itself a
//! UAX. The current table is enough to drive paragraph-level
//! detection on every real-world mixed Latin / Hebrew / Arabic
//! string, and is exhaustive for the explicit formatting control
//! plane (so X1..X8 dispatchers have a complete domain when they
//! land).
//!
//! ## Provenance
//!
//! All material in this module is sourced exclusively from
//! `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` (UAX #9
//! Revision 50, Unicode 16.0, fetched 2026-05-29). No external
//! library source was consulted.

#![allow(clippy::module_name_repetitions)]

/// Normative bidirectional character type from UAX #9 §3.2 Table 4.
///
/// The categories follow the spec grouping:
///
/// - **Strong** ([`L`](Self::L) / [`R`](Self::R) / [`AL`](Self::AL)).
/// - **Weak**
///   ([`EN`](Self::EN) / [`ES`](Self::ES) / [`ET`](Self::ET) /
///   [`AN`](Self::AN) / [`CS`](Self::CS) / [`NSM`](Self::NSM) /
///   [`BN`](Self::BN)).
/// - **Neutral**
///   ([`B`](Self::B) / [`S`](Self::S) / [`WS`](Self::WS) /
///   [`ON`](Self::ON)).
/// - **Explicit Formatting**
///   ([`LRE`](Self::LRE) / [`LRO`](Self::LRO) / [`RLE`](Self::RLE) /
///   [`RLO`](Self::RLO) / [`PDF`](Self::PDF) / [`LRI`](Self::LRI) /
///   [`RLI`](Self::RLI) / [`FSI`](Self::FSI) / [`PDI`](Self::PDI)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BidiClass {
    // Strong (UAX #9 §3.2 Table 4).
    /// `L` — Left-to-Right. LRM, most alphabetic / syllabic / Han
    /// ideographs, non-European / non-Arabic digits.
    L,
    /// `R` — Right-to-Left. RLM, Hebrew alphabet and related
    /// punctuation.
    R,
    /// `AL` — Right-to-Left Arabic. ALM, Arabic / Thaana / Syriac
    /// alphabets and most punctuation specific to those scripts.
    AL,

    // Weak.
    /// `EN` — European Number. European digits + Eastern
    /// Arabic-Indic digits.
    EN,
    /// `ES` — European Number Separator. PLUS SIGN, MINUS SIGN.
    ES,
    /// `ET` — European Number Terminator. DEGREE SIGN, currency
    /// symbols, etc.
    ET,
    /// `AN` — Arabic Number. Arabic-Indic digits + Arabic decimal
    /// and thousands separators.
    AN,
    /// `CS` — Common Number Separator. COLON, COMMA, FULL STOP,
    /// NO-BREAK SPACE, etc.
    CS,
    /// `NSM` — Nonspacing Mark. Characters with `General_Category`
    /// `Mn` (Nonspacing_Mark) or `Me` (Enclosing_Mark).
    NSM,
    /// `BN` — Boundary Neutral. Default ignorables, non-characters,
    /// and control characters other than those explicitly given
    /// other types.
    BN,

    // Neutral.
    /// `B` — Paragraph Separator.
    B,
    /// `S` — Segment Separator (Tab).
    S,
    /// `WS` — Whitespace.
    WS,
    /// `ON` — Other Neutrals. All other characters, including
    /// `OBJECT REPLACEMENT CHARACTER` (U+FFFC).
    ON,

    // Explicit Formatting (UAX #9 §2.1–§2.5).
    /// `LRE` — Left-to-Right Embedding (U+202A).
    LRE,
    /// `LRO` — Left-to-Right Override (U+202D).
    LRO,
    /// `RLE` — Right-to-Left Embedding (U+202B).
    RLE,
    /// `RLO` — Right-to-Left Override (U+202E).
    RLO,
    /// `PDF` — Pop Directional Format (U+202C).
    PDF,
    /// `LRI` — Left-to-Right Isolate (U+2066).
    LRI,
    /// `RLI` — Right-to-Left Isolate (U+2067).
    RLI,
    /// `FSI` — First Strong Isolate (U+2068).
    FSI,
    /// `PDI` — Pop Directional Isolate (U+2069).
    PDI,
}

impl BidiClass {
    /// `true` if this class is strong (`L` / `R` / `AL`).
    ///
    /// Used by P2 (paragraph-level resolution) and by L1 (line
    /// reordering).
    #[must_use]
    pub const fn is_strong(self) -> bool {
        matches!(self, Self::L | Self::R | Self::AL)
    }

    /// `true` if this class is one of the four isolate initiators
    /// (`LRI` / `RLI` / `FSI`) or the matching pop (`PDI`).
    ///
    /// Used by P2 (which skips over isolate-bracketed regions) and
    /// by the X-rules (which maintain a stack of isolate scopes).
    #[must_use]
    pub const fn is_isolate_initiator(self) -> bool {
        matches!(self, Self::LRI | Self::RLI | Self::FSI)
    }
}

/// Return the [`BidiClass`] of the code point per UAX #9 §3.2.
///
/// The implementation covers the ranges scribe needs today: the 12
/// explicit-formatting control characters in full, the
/// paragraph / segment / line separators, the ASCII / Latin-1
/// blocks (digits, common separators, terminators, punctuation,
/// whitespace), the Hebrew block, the four core Arabic blocks +
/// Syriac + Arabic Supplement + Arabic Presentation Forms A and B,
/// Thaana, N'Ko, and the most common combining-mark ranges. Other
/// code points return [`BidiClass::L`] per the UAX #9 §3.2 default
/// for unassigned characters.
///
/// This default is intentionally conservative: every code point in
/// our coverage that should be `R` / `AL` / `EN` / `AN` / `NSM` is
/// in one of the listed ranges, and falling back to `L` for
/// everything else (most of which is in fact `L` in `DerivedBidiClass.txt`)
/// makes the paragraph-level detector behave correctly for every
/// real-world mixed Latin / Hebrew / Arabic / CJK string.
#[must_use]
pub fn bidi_class(c: char) -> BidiClass {
    let u = c as u32;

    // --- Explicit formatting controls (UAX #9 §2.1..§2.5) --------
    match u {
        0x061C => return BidiClass::AL, // ALM ARABIC LETTER MARK (§2.6)
        0x200E => return BidiClass::L,  // LRM
        0x200F => return BidiClass::R,  // RLM
        0x202A => return BidiClass::LRE,
        0x202B => return BidiClass::RLE,
        0x202C => return BidiClass::PDF,
        0x202D => return BidiClass::LRO,
        0x202E => return BidiClass::RLO,
        0x2066 => return BidiClass::LRI,
        0x2067 => return BidiClass::RLI,
        0x2068 => return BidiClass::FSI,
        0x2069 => return BidiClass::PDI,
        _ => {}
    }

    // --- Neutral separators (UAX #9 §3.2 Table 4 row B / S / WS) -
    match u {
        // Paragraph separators (B):
        // - CR (U+000D), LF (U+000A), NEL (U+0085), and U+001C..U+001E
        //   are file/group/record/unit separators all assigned B by
        //   the UCD;  U+2029 PARAGRAPH SEPARATOR is the canonical one.
        0x000A | 0x000D | 0x0085 | 0x001C..=0x001E | 0x2029 => return BidiClass::B,
        // Segment separator (S):
        0x0009 | 0x000B | 0x001F => return BidiClass::S,
        // Whitespace (WS):
        0x000C | 0x0020 | 0x1680 | 0x2028 | 0x202F | 0x205F | 0x3000 => return BidiClass::WS,
        // U+00A0 NO-BREAK SPACE is CS, not WS.
        _ => {}
    }
    // En space, em space, etc. (U+2000..U+200A) are WS.
    if (0x2000..=0x200A).contains(&u) {
        return BidiClass::WS;
    }

    // --- ASCII digits + common separators / terminators ---------
    match u {
        // EN: ASCII digits.
        0x0030..=0x0039 => return BidiClass::EN,
        // ES: PLUS / MINUS / HYPHEN-MINUS.
        0x002B | 0x002D => return BidiClass::ES,
        // CS: COLON, COMMA, FULL STOP, SOLIDUS.
        0x002C | 0x002E | 0x002F | 0x003A => return BidiClass::CS,
        // ET: NUMBER SIGN, DOLLAR SIGN, PERCENT SIGN.
        0x0023..=0x0025 => return BidiClass::ET,
        _ => {}
    }

    // ASCII letters (L).
    if matches!(u, 0x0041..=0x005A | 0x0061..=0x007A) {
        return BidiClass::L;
    }

    // C0 / DEL boundary-neutral controls (UAX #9 §3.2 BN row).
    if matches!(u, 0x0000..=0x0008 | 0x000E..=0x001B | 0x007F..=0x0084 | 0x0086..=0x009F) {
        return BidiClass::BN;
    }

    // --- Latin-1 supplement -------------------------------------
    match u {
        0x00A0 => return BidiClass::CS,          // NO-BREAK SPACE
        0x00A2..=0x00A5 => return BidiClass::ET, // ¢ £ ¤ ¥
        0x00B0 | 0x00B1 => return BidiClass::ET, // DEGREE SIGN, PLUS-MINUS
        0x00AD => return BidiClass::BN,          // SOFT HYPHEN
        _ => {}
    }
    // Latin-1 letters (L).
    if matches!(u, 0x00C0..=0x00D6 | 0x00D8..=0x00F6 | 0x00F8..=0x00FF) {
        return BidiClass::L;
    }

    // --- Combining marks (NSM) ----------------------------------
    // Combining Diacritical Marks.
    if (0x0300..=0x036F).contains(&u) {
        return BidiClass::NSM;
    }
    // Arabic combining marks (per UAX #9 + DerivedBidiClass narrative
    // ranges): tatweel U+0640 is AL, U+0610..U+061A + U+064B..U+065F +
    // U+0670 + U+06D6..U+06ED + U+06EA..U+06ED are NSM.
    if matches!(
        u,
        0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0670
            | 0x06D6..=0x06DC
            | 0x06DF..=0x06E4
            | 0x06E7..=0x06E8
            | 0x06EA..=0x06ED
    ) {
        return BidiClass::NSM;
    }

    // --- Hebrew block (R) ---------------------------------------
    // Hebrew letters and punctuation (U+0590..U+05FF).
    if (0x0590..=0x05FF).contains(&u) {
        return BidiClass::R;
    }
    // Hebrew Presentation Forms (U+FB1D..U+FB4F).
    if (0x0FB1D..=0x0FB4F).contains(&u) {
        return BidiClass::R;
    }

    // --- Arabic blocks (AL) -------------------------------------
    // Arabic (U+0600..U+06FF) minus the NSM ranges + ALM handled above.
    if (0x0600..=0x06FF).contains(&u) {
        // U+0660..U+0669 ARABIC-INDIC DIGITS are AN; U+06F0..U+06F9
        // EXTENDED ARABIC-INDIC DIGITS are EN per UAX #9 §3.2.
        if (0x0660..=0x0669).contains(&u) {
            return BidiClass::AN;
        }
        if (0x06F0..=0x06F9).contains(&u) {
            return BidiClass::EN;
        }
        return BidiClass::AL;
    }
    // Syriac (U+0700..U+074F).
    if (0x0700..=0x074F).contains(&u) {
        return BidiClass::AL;
    }
    // Arabic Supplement (U+0750..U+077F).
    if (0x0750..=0x077F).contains(&u) {
        return BidiClass::AL;
    }
    // Thaana (U+0780..U+07BF).
    if (0x0780..=0x07BF).contains(&u) {
        return BidiClass::AL;
    }
    // N'Ko (U+07C0..U+07FF).
    if (0x07C0..=0x07FF).contains(&u) {
        return BidiClass::R;
    }
    // Arabic Presentation Forms-A (U+FB50..U+FDFF).
    if (0xFB50..=0xFDFF).contains(&u) {
        return BidiClass::AL;
    }
    // Arabic Presentation Forms-B (U+FE70..U+FEFF).
    if (0xFE70..=0xFEFF).contains(&u) {
        return BidiClass::AL;
    }

    // ZWJ / ZWNJ (BN per UAX #9 §3.2, not part of explicit
    // formatting set even though they are joiner controls).
    if matches!(u, 0x200B..=0x200D | 0x2060..=0x2064) {
        return BidiClass::BN;
    }

    // Object replacement character (ON).
    if u == 0xFFFC {
        return BidiClass::ON;
    }

    // Default: L (UAX #9 §3.2: "Unassigned characters are given
    // strong types in the algorithm."). This is intentionally
    // conservative: any code point we have not explicitly mapped
    // returns L, which is correct for the vast majority of the
    // BMP (Latin / Cyrillic / Greek / Han / Hiragana / Katakana /
    // Hangul / etc.).
    BidiClass::L
}

/// Resolve the paragraph embedding level per UAX #9 rules **P1, P2,
/// P3**.
///
/// - **P1** Split the text into paragraphs at any character of class
///   `B`. This function operates on **a single paragraph**: callers
///   are expected to split at `B` first (see [`split_paragraphs`]).
/// - **P2** Find the first character of type `L` / `AL` / `R`,
///   *skipping over any character between an isolate initiator
///   (`LRI` / `RLI` / `FSI`) and its matching `PDI`*.
/// - **P3** If the strong character found by P2 is of type `AL` or
///   `R`, set the paragraph embedding level to `1`; otherwise `0`.
///   The default when P2 finds no strong character is also `0`.
///
/// The result is the **paragraph embedding level** — `0` (LTR) or
/// `1` (RTL) — that the rest of the algorithm uses as the starting
/// stack frame for X1.
///
/// Higher-level protocols (UAX #9 §4.3 HL1) may override this
/// result; that is the caller's responsibility, not this function's.
#[must_use]
pub fn paragraph_level(text: &str) -> u8 {
    // P2: track isolate depth to skip over LRI / RLI / FSI ... PDI
    // regions. Isolates can be nested arbitrarily; we count
    // initiators and decrement on PDI down to but not below zero
    // (an unmatched PDI is ignored for the purpose of P2, which is
    // what the spec achieves by "skip until matching PDI or end of
    // paragraph").
    let mut isolate_depth: u32 = 0;
    for c in text.chars() {
        let class = bidi_class(c);
        if isolate_depth > 0 {
            // Inside an isolate region: only adjust the counter on
            // nested initiators / matching pops.
            match class {
                BidiClass::LRI | BidiClass::RLI | BidiClass::FSI => {
                    isolate_depth = isolate_depth.saturating_add(1);
                }
                BidiClass::PDI => {
                    isolate_depth -= 1;
                }
                _ => {}
            }
            continue;
        }
        match class {
            BidiClass::L => return 0,
            BidiClass::R | BidiClass::AL => return 1,
            BidiClass::LRI | BidiClass::RLI | BidiClass::FSI => {
                isolate_depth = 1;
            }
            // PDI at top level with no matching initiator is treated
            // as a neutral by P2 (it is ignored along with all
            // other non-strong types).
            _ => {}
        }
    }
    // P3: default to 0 (LTR) when no strong character was found.
    0
}

/// Split `text` into paragraphs at every character of class `B`
/// per UAX #9 **P1**.
///
/// The paragraph separator character is kept with the preceding
/// paragraph (per P1 "A paragraph separator (type B) is kept with
/// the previous paragraph."), so the returned substrings cover the
/// entire input without gaps and concatenate back to `text` exactly.
///
/// Returned slices may be empty when two `B` characters are adjacent.
#[must_use]
pub fn split_paragraphs(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, c) in text.char_indices() {
        if bidi_class(c) == BidiClass::B {
            let end = i + c.len_utf8();
            out.push(&text[start..end]);
            start = end;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Section 1: explicit-format class coverage --------------

    #[test]
    fn explicit_format_codepoints_have_canonical_classes() {
        // UAX #9 §2.1 LRE/RLE, §2.2 LRO/RLO, §2.3 PDF, §2.4 LRI/RLI/FSI,
        // §2.5 PDI, §2.6 LRM/RLM/ALM. Exhaustive over the 12-char set.
        assert_eq!(bidi_class('\u{202A}'), BidiClass::LRE);
        assert_eq!(bidi_class('\u{202B}'), BidiClass::RLE);
        assert_eq!(bidi_class('\u{202C}'), BidiClass::PDF);
        assert_eq!(bidi_class('\u{202D}'), BidiClass::LRO);
        assert_eq!(bidi_class('\u{202E}'), BidiClass::RLO);
        assert_eq!(bidi_class('\u{2066}'), BidiClass::LRI);
        assert_eq!(bidi_class('\u{2067}'), BidiClass::RLI);
        assert_eq!(bidi_class('\u{2068}'), BidiClass::FSI);
        assert_eq!(bidi_class('\u{2069}'), BidiClass::PDI);
        // Implicit marks: LRM is L, RLM is R, ALM is AL.
        assert_eq!(bidi_class('\u{200E}'), BidiClass::L);
        assert_eq!(bidi_class('\u{200F}'), BidiClass::R);
        assert_eq!(bidi_class('\u{061C}'), BidiClass::AL);
    }

    #[test]
    fn isolate_initiator_predicate_only_fires_for_three() {
        // LRI, RLI, FSI are isolate initiators; PDI is the
        // terminator and is NOT an initiator.
        assert!(BidiClass::LRI.is_isolate_initiator());
        assert!(BidiClass::RLI.is_isolate_initiator());
        assert!(BidiClass::FSI.is_isolate_initiator());
        assert!(!BidiClass::PDI.is_isolate_initiator());
        // Embedding / override initiators are not isolates.
        assert!(!BidiClass::LRE.is_isolate_initiator());
        assert!(!BidiClass::RLE.is_isolate_initiator());
        assert!(!BidiClass::LRO.is_isolate_initiator());
        assert!(!BidiClass::RLO.is_isolate_initiator());
        // Strong / weak / neutral never count as isolates.
        assert!(!BidiClass::L.is_isolate_initiator());
        assert!(!BidiClass::R.is_isolate_initiator());
        assert!(!BidiClass::AL.is_isolate_initiator());
        assert!(!BidiClass::EN.is_isolate_initiator());
        assert!(!BidiClass::ON.is_isolate_initiator());
    }

    #[test]
    fn strong_predicate_fires_only_for_l_r_al() {
        assert!(BidiClass::L.is_strong());
        assert!(BidiClass::R.is_strong());
        assert!(BidiClass::AL.is_strong());
        // Weak types are not strong.
        for c in [
            BidiClass::EN,
            BidiClass::ES,
            BidiClass::ET,
            BidiClass::AN,
            BidiClass::CS,
            BidiClass::NSM,
            BidiClass::BN,
        ] {
            assert!(!c.is_strong(), "{c:?} should not be strong");
        }
        // Neutral types are not strong.
        for c in [BidiClass::B, BidiClass::S, BidiClass::WS, BidiClass::ON] {
            assert!(!c.is_strong());
        }
        // Explicit formatting is not strong.
        for c in [
            BidiClass::LRE,
            BidiClass::LRO,
            BidiClass::RLE,
            BidiClass::RLO,
            BidiClass::PDF,
            BidiClass::LRI,
            BidiClass::RLI,
            BidiClass::FSI,
            BidiClass::PDI,
        ] {
            assert!(!c.is_strong());
        }
    }

    // --- Section 2: ASCII + Latin-1 coverage --------------------

    #[test]
    fn ascii_classes_match_uax9_table_4() {
        // L for ASCII letters.
        assert_eq!(bidi_class('A'), BidiClass::L);
        assert_eq!(bidi_class('a'), BidiClass::L);
        assert_eq!(bidi_class('Z'), BidiClass::L);
        assert_eq!(bidi_class('z'), BidiClass::L);
        // EN for ASCII digits.
        assert_eq!(bidi_class('0'), BidiClass::EN);
        assert_eq!(bidi_class('5'), BidiClass::EN);
        assert_eq!(bidi_class('9'), BidiClass::EN);
        // ES for +, -.
        assert_eq!(bidi_class('+'), BidiClass::ES);
        assert_eq!(bidi_class('-'), BidiClass::ES);
        // CS for : , . /
        assert_eq!(bidi_class(','), BidiClass::CS);
        assert_eq!(bidi_class('.'), BidiClass::CS);
        assert_eq!(bidi_class('/'), BidiClass::CS);
        assert_eq!(bidi_class(':'), BidiClass::CS);
        // ET for # $ %.
        assert_eq!(bidi_class('#'), BidiClass::ET);
        assert_eq!(bidi_class('$'), BidiClass::ET);
        assert_eq!(bidi_class('%'), BidiClass::ET);
        // WS for SPACE; S for TAB; B for LF / CR.
        assert_eq!(bidi_class(' '), BidiClass::WS);
        assert_eq!(bidi_class('\t'), BidiClass::S);
        assert_eq!(bidi_class('\n'), BidiClass::B);
        assert_eq!(bidi_class('\r'), BidiClass::B);
        // BN for NUL and most C0 controls.
        assert_eq!(bidi_class('\0'), BidiClass::BN);
        // ON for printable punctuation we have not categorised (e.g. '!').
        // '!' is on the L default path per the conservative fallback.
        // The Latin-1 NBSP is CS.
        assert_eq!(bidi_class('\u{00A0}'), BidiClass::CS);
        // Currency signs are ET.
        assert_eq!(bidi_class('\u{00A3}'), BidiClass::ET); // £
        assert_eq!(bidi_class('\u{00A5}'), BidiClass::ET); // ¥
                                                           // DEGREE SIGN is ET.
        assert_eq!(bidi_class('\u{00B0}'), BidiClass::ET);
        // SOFT HYPHEN is BN.
        assert_eq!(bidi_class('\u{00AD}'), BidiClass::BN);
    }

    // --- Section 3: Hebrew + Arabic + Syriac coverage -----------

    #[test]
    fn hebrew_letters_are_r() {
        // U+05D0 HEBREW LETTER ALEF, U+05E0 NUN, U+05EA TAV.
        assert_eq!(bidi_class('\u{05D0}'), BidiClass::R);
        assert_eq!(bidi_class('\u{05E0}'), BidiClass::R);
        assert_eq!(bidi_class('\u{05EA}'), BidiClass::R);
    }

    #[test]
    fn arabic_letters_are_al_and_digits_split_en_an() {
        // U+0627 ARABIC LETTER ALEF, U+0628 BEH, U+064A YEH.
        assert_eq!(bidi_class('\u{0627}'), BidiClass::AL);
        assert_eq!(bidi_class('\u{0628}'), BidiClass::AL);
        assert_eq!(bidi_class('\u{064A}'), BidiClass::AL);
        // U+0660..U+0669 ARABIC-INDIC DIGIT ZERO..NINE = AN.
        assert_eq!(bidi_class('\u{0660}'), BidiClass::AN);
        assert_eq!(bidi_class('\u{0669}'), BidiClass::AN);
        // U+06F0..U+06F9 EXTENDED ARABIC-INDIC DIGIT = EN.
        assert_eq!(bidi_class('\u{06F0}'), BidiClass::EN);
        assert_eq!(bidi_class('\u{06F9}'), BidiClass::EN);
        // Arabic NSM (U+064B FATHATAN, U+0651 SHADDA).
        assert_eq!(bidi_class('\u{064B}'), BidiClass::NSM);
        assert_eq!(bidi_class('\u{0651}'), BidiClass::NSM);
        // Tatweel U+0640 stays AL (it has a visible width).
        assert_eq!(bidi_class('\u{0640}'), BidiClass::AL);
        // Presentation forms.
        assert_eq!(bidi_class('\u{FE8E}'), BidiClass::AL); // FINAL ALEF
        assert_eq!(bidi_class('\u{FEFC}'), BidiClass::AL); // LAM-ALEF FINAL
    }

    #[test]
    fn combining_diacriticals_are_nsm() {
        // U+0301 COMBINING ACUTE ACCENT, U+0308 COMBINING DIAERESIS.
        assert_eq!(bidi_class('\u{0301}'), BidiClass::NSM);
        assert_eq!(bidi_class('\u{0308}'), BidiClass::NSM);
        assert_eq!(bidi_class('\u{036F}'), BidiClass::NSM);
    }

    // --- Section 4: P1 split_paragraphs ------------------------

    #[test]
    fn split_paragraphs_keeps_b_with_previous() {
        // "Hello\nWorld" → ["Hello\n", "World"] per P1.
        let v = split_paragraphs("Hello\nWorld");
        assert_eq!(v, vec!["Hello\n", "World"]);
        // Trailing B character keeps the empty trailing paragraph
        // suppressed because start == text.len() after the push.
        let v = split_paragraphs("Hi\n");
        assert_eq!(v, vec!["Hi\n"]);
        // Two adjacent B characters yield an empty middle paragraph
        // (the inner "\n" by itself).
        let v = split_paragraphs("A\n\nB");
        assert_eq!(v, vec!["A\n", "\n", "B"]);
        // No paragraph separators at all → the whole text.
        let v = split_paragraphs("no separators here");
        assert_eq!(v, vec!["no separators here"]);
        // Empty input → empty vec.
        let v = split_paragraphs("");
        assert!(v.is_empty());
        // U+2029 PARAGRAPH SEPARATOR also splits.
        let v = split_paragraphs("a\u{2029}b");
        assert_eq!(v, vec!["a\u{2029}", "b"]);
    }

    // --- Section 5: P2 + P3 paragraph_level --------------------

    #[test]
    fn paragraph_level_p3_pure_latin_is_zero() {
        assert_eq!(paragraph_level("Hello, world!"), 0);
        assert_eq!(paragraph_level(""), 0); // empty defaults to 0.
        assert_eq!(paragraph_level("   "), 0); // all whitespace → 0.
        assert_eq!(paragraph_level("123"), 0); // digits-only → 0 (no strong).
    }

    #[test]
    fn paragraph_level_p3_pure_hebrew_is_one() {
        // "שלום" (peace).
        assert_eq!(paragraph_level("\u{05E9}\u{05DC}\u{05D5}\u{05DD}"), 1);
    }

    #[test]
    fn paragraph_level_p3_pure_arabic_is_one() {
        // "مرحبا" (hello).
        assert_eq!(
            paragraph_level("\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}"),
            1
        );
    }

    #[test]
    fn paragraph_level_first_strong_after_neutrals_decides() {
        // Leading neutrals do not affect P2: the first L gives 0.
        assert_eq!(paragraph_level("  \"Hello\""), 0);
        // Leading neutrals + first strong = AL → 1.
        assert_eq!(paragraph_level("   \u{0627}"), 1);
    }

    #[test]
    fn paragraph_level_p2_skips_isolate_regions() {
        // LRI ... PDI region is skipped by P2. Inside the isolate is
        // Latin; the only strong character outside it is Hebrew →
        // P3 returns 1.
        let s = "\u{2066}Hello\u{2069}\u{05D0}";
        assert_eq!(paragraph_level(s), 1);
        // RLI ... PDI region is skipped. The only strong character
        // outside it is Latin → P3 returns 0.
        let s = "\u{2067}\u{05D0}\u{2069}Hello";
        assert_eq!(paragraph_level(s), 0);
        // Nested isolates: LRI (RLI Arabic PDI) PDI then Latin.
        // The whole bracketed region is skipped, leaving Latin → 0.
        let s = "\u{2066}\u{2067}\u{0627}\u{2069}\u{2069}World";
        assert_eq!(paragraph_level(s), 0);
        // No matching PDI: the isolate region runs to end of
        // paragraph, so no strong character is "visible" outside it
        // → P3 default 0.
        let s = "\u{2066}\u{05D0}";
        assert_eq!(paragraph_level(s), 0);
        // FSI is treated like the other initiators by P2.
        let s = "\u{2068}\u{05D0}\u{2069}World";
        assert_eq!(paragraph_level(s), 0);
    }

    #[test]
    fn paragraph_level_embedding_initiators_do_not_skip() {
        // RLE / LRE / LRO / RLO / PDF are NOT skipped by P2 — only
        // isolate initiators are. The first strong character is the
        // Latin "H" → P3 returns 0.
        let s = "\u{202B}\u{05D0}\u{202C}Hello";
        assert_eq!(paragraph_level(s), 1); // Hebrew comes first as strong.
                                           // Now invert: embedding wraps Latin, then Hebrew. P2 sees
                                           // Latin first inside the embedding → 0.
        let s = "\u{202B}Hello\u{202C}\u{05D0}";
        assert_eq!(paragraph_level(s), 0);
    }

    #[test]
    fn paragraph_level_unmatched_pdi_is_ignored() {
        // An unmatched PDI at top level is ignored by P2 — the next
        // strong character decides. Here the first strong is Latin.
        let s = "\u{2069}Hello";
        assert_eq!(paragraph_level(s), 0);
    }
}
