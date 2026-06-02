//! Unicode Bidirectional Algorithm — UAX #9 character classes,
//! paragraph-level resolution (rules P1 / P2 / P3), weak-type
//! resolution (rules W1..W7), neutral-type resolution (rules N1
//! and N2 — bracket-pair rule N0 deferred to a follow-up round),
//! implicit-level resolution (rules I1 / I2), and line-level
//! reordering (rules L1 / L2).
//!
//! ## Scope
//!
//! This module implements the **paragraph + weak-type phases** of the
//! Unicode Bidirectional Algorithm (UBA) as specified in Unicode
//! Standard Annex #9, *Unicode Bidirectional Algorithm*, Revision 50
//! / Unicode 16.0 (the dated snapshot pinned at
//! `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`). The surface
//! is:
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
//! - [`resolve_weak_types`] — the **W1..W7** rules from §3.3.4
//!   applied to one isolating run sequence in place: NSM type
//!   inheritance (W1), `EN` after `AL` strong → `AN` (W2), `AL` →
//!   `R` (W3), single-separator-between-two-numbers collapse (W4),
//!   `ET`-adjacent-to-`EN` collapse (W5), leftover-separator
//!   neutralisation (W6), and `EN` after `L` → `L` (W7). The phase
//!   leaves the slice with no `AL` (collapsed to `R`) and no
//!   leftover `ES` / `ET` / `CS` (collapsed to `ON`), so the
//!   N-rules can resolve neutrals against a clean weak-type
//!   vocabulary.
//! - [`resolve_neutral_types`] — the **N1 + N2** rules from §3.3.5
//!   applied to one isolating run sequence already passed through
//!   `resolve_weak_types`. N1 walks every maximal run of Neutral-or-
//!   Isolate-formatting (NI) elements (`B` / `S` / `WS` / `ON` /
//!   `LRI` / `RLI` / `FSI` / `PDI`) and, when the strong type on
//!   both sides (counting `EN` / `AN` as `R`, and `sos` / `eos` at
//!   the sequence boundaries) is the same, flips every NI in the
//!   run to that strong type (`L` or `R`). N2 fills the remaining
//!   NIs with the **embedding direction** derived from the caller-
//!   provided embedding level (even → `L`, odd → `R`). After the
//!   call the slice contains no NI: every former neutral or isolate
//!   formatting character has been resolved to a strong direction,
//!   ready for the §3.3.6 implicit-level pass (I1 / I2).
//! - [`resolve_implicit_levels`] — the **I1 + I2** rules from §3.3.6
//!   applied to one isolating run sequence already passed through
//!   `resolve_neutral_types`. I1 bumps `R` by +1 and `EN` / `AN` by
//!   +2 above an even embedding level; I2 bumps `L` / `EN` / `AN`
//!   by +1 above an odd embedding level. The two together implement
//!   UAX #9 Table 5 verbatim. `BN` is ignored per §5.2 ("In rules
//!   I1 and I2, ignore BN.") — its level stays at the embedding
//!   level so a follow-up L-rule pass can fold it. The function
//!   returns a `Vec<u8>` of per-character resolved levels, ready
//!   for the L-rule reordering pass.
//! - [`reset_trailing_levels`] — the **L1** rule from §3.4. Walks
//!   the line and, in place, resets the embedding level of every
//!   segment separator (`S`), every paragraph separator (`B`), and
//!   every maximal trailing run of whitespace (`WS`) + isolate
//!   formatting (`LRI` / `RLI` / `FSI` / `PDI`) immediately
//!   preceding such a separator or at the end of the line, back to
//!   the paragraph embedding level. Per UAX #9 the lookup uses the
//!   **original** bidi classes of the line — the caller passes the
//!   input class slice alongside the post-I-rules level vector.
//! - [`reorder_line`] — the **L2** rule from §3.4. Returns a
//!   permutation of `0..n` mapping visual position to logical
//!   index, computed by the progressive-reversal algorithm: from
//!   the maximum level down to the smallest odd level, reverse
//!   every maximal contiguous run of characters whose level is at
//!   least the iteration level. The output drives the
//!   logical-to-visual remap a renderer applies before rasterising
//!   the glyph sequence.
//!
//! ## Out of scope (deferred to follow-up rounds)
//!
//! - N0 (bracket-pair resolution per §3.1.3 + §3.3.5); requires the
//!   Unicode `BidiBrackets.txt` data file to identify opening /
//!   closing paired brackets, which is not yet vendored under
//!   `docs/`.
//! - X1..X10 (explicit embedding / override / isolate stack
//!   machinery + the isolating-run-sequence partition).
//! - L3 (combining-mark reordering for RTL bases). The rule is
//!   conditional on the rendering engine's mark-attachment policy
//!   per UAX #9 §3.4: "If the rendering engine expects them to
//!   follow the base characters in the final display process, then
//!   the ordering of the marks and the base character must be
//!   reversed." Scribe's GPOS mark-to-base + mark-to-mark stacker
//!   keeps the logical (post-base) order in both directions, so
//!   the conditional does not fire today; revisit when a CTL
//!   pipeline using glyph-spacing overhangs lands.
//! - L4 (mirroring of bidi-mirrored characters at R-resolved
//!   levels per UAX #9 §3.4 / §4.7); requires the Unicode
//!   `BidiMirroring.txt` / `Bidi_Mirrored` data file to identify
//!   the mirrored set + their mirror pair, which is not yet
//!   vendored under `docs/` alongside the UAX HTML.
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
//! Revision 50, Unicode 16.0, fetched 2026-05-29).

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

    /// `true` if this class counts as a **Neutral or Isolate (NI)**
    /// in UAX #9 §3.3.5 / §3.3.6 terminology.
    ///
    /// The NI alias names the union `B | S | WS | ON | FSI | LRI | RLI
    /// | PDI` — every neutral type plus the four isolate-formatting
    /// characters, which are *treated as if neutral* once W1..W7 have
    /// resolved their surroundings. W7 uses the NI alias in its
    /// "search backward through NIs" wording; the N-rules (N0..N2)
    /// resolve NIs en masse in the next phase.
    #[must_use]
    pub const fn is_neutral_or_isolate(self) -> bool {
        matches!(
            self,
            Self::B | Self::S | Self::WS | Self::ON | Self::FSI | Self::LRI | Self::RLI | Self::PDI
        )
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

/// Resolve **weak types** for one isolating run sequence per UAX #9
/// **W1, W2, W3, W4, W5, W6, W7** (§3.3.4).
///
/// The input `classes` are the per-character [`BidiClass`] values for
/// **one isolating run sequence** in logical order. `sos` is the
/// **start-of-sequence** strong type (`L` or `R`) — for callers that
/// have not yet wired X1..X10 / X10's run partition, passing
/// `L` (paragraph level 0) or `R` (paragraph level 1) is correct for
/// a single-paragraph, no-isolate input. `eos` is the **end-of-
/// sequence** strong type, also `L` or `R`. Only W2 + W7 read `sos`
/// (W7 needs only `L` / `R` / `sos`); none of the rules read `eos`
/// directly in this single-pass implementation (W4 reads the
/// *following* character, but only when that character is *inside*
/// the sequence — at the trailing edge the "single-separator-between-
/// two-EN" pattern cannot apply because there is no following EN).
///
/// The function mutates `classes` in place. After return every
/// element is one of `L`, `R`, `EN`, `AN`, `NSM`, `ES`, `ET`, `CS`,
/// `BN`, or one of the neutral / isolate-formatting types
/// (`B` / `S` / `WS` / `ON` / `LRI` / `RLI` / `FSI` / `PDI`) — `AL`
/// is gone (W3 collapses every remaining AL to R) and every
/// separator / terminator that survived W4 / W5 is collapsed by W6
/// to `ON`. The N-rules pick up from there.
///
/// The implementation is the literal four-pass shape from the spec:
///
/// 1. **W1** — NSMs take the type of the previous character (or
///    `ON` if the previous is `LRI` / `RLI` / `FSI` / `PDI`, per the
///    spec note about "isolate initiator or PDI"). An NSM at the
///    start of the sequence takes the `sos` type.
/// 2. **W2** — EN immediately after the most-recent strong of type
///    `AL` becomes `AN`. The "most-recent strong" walk includes
///    `sos` as the implicit start-of-sequence strong type.
/// 3. **W3** — every `AL` becomes `R`.
/// 4. **W4** — single `ES` between two `EN`s becomes `EN`; single
///    `CS` between two `EN`s becomes `EN`; single `CS` between two
///    `AN`s becomes `AN`.
/// 5. **W5** — runs of `ET` adjacent (on either side) to `EN` become
///    `EN`.
/// 6. **W6** — every remaining `ES` / `ET` / `CS` becomes `ON`.
/// 7. **W7** — `EN` whose most-recent strong (among `L` / `R` /
///    `sos`, **not** `AL` because W3 already turned every `AL` into
///    `R`) is `L` becomes `L`.
///
/// Provenance: rules transcribed verbatim from
/// `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` §3.3.4 (UAX
/// #9 Revision 50, Unicode 16.0).
///
/// # Panics
///
/// Does not panic. Empty input is a no-op.
///
/// # Examples
///
/// ```
/// use oxideav_scribe::bidi::{resolve_weak_types, BidiClass};
///
/// // "AL EN" with sos=L: W2 sees the AL as the most-recent strong,
/// // so EN → AN; then W3 turns the AL into R. Final: [R, AN].
/// let mut cls = vec![BidiClass::AL, BidiClass::EN];
/// resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
/// assert_eq!(cls, vec![BidiClass::R, BidiClass::AN]);
///
/// // "L NI EN" → W7 sees L as the most-recent strong, so EN → L.
/// let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::EN];
/// resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
/// assert_eq!(cls, vec![BidiClass::L, BidiClass::ON, BidiClass::L]);
/// ```
pub fn resolve_weak_types(classes: &mut [BidiClass], sos: BidiClass, eos: BidiClass) {
    let _ = eos; // eos is not consumed by W1..W7 (kept in the signature
                 // for symmetry with the N-rules + because the spec
                 // narration references it for boundary cases).
    if classes.is_empty() {
        return;
    }

    // --- W1: NSM takes the type of the previous character ---------
    //
    // Per spec: "Examine each nonspacing mark (NSM) in the isolating
    // run sequence, and change the type of the NSM to Other Neutral
    // if the previous character is an isolate initiator or PDI, and
    // to the type of the previous character otherwise. If the NSM is
    // at the start of the isolating run sequence, it will get the
    // type of sos." The examples in the spec confirm: AL NSM NSM →
    // AL AL AL (consecutive NSMs all flip to the same type because
    // the second NSM, after W1's first iteration, sees a previously-
    // rewritten AL).
    for i in 0..classes.len() {
        if classes[i] != BidiClass::NSM {
            continue;
        }
        let prev = if i == 0 { sos } else { classes[i - 1] };
        classes[i] = match prev {
            BidiClass::LRI | BidiClass::RLI | BidiClass::FSI | BidiClass::PDI => BidiClass::ON,
            other => other,
        };
    }

    // --- W2: EN preceded (going backward) by AL becomes AN --------
    //
    // Per spec: "Search backward from each instance of a European
    // number until the first strong type (R, L, AL, or sos) is
    // found. If an AL is found, change the type of the European
    // number to Arabic number." Implementation: a forward sweep that
    // tracks the most recent strong (including sos) and rewrites EN
    // → AN when that strong is AL.
    {
        let mut last_strong = if sos.is_strong() { sos } else { BidiClass::L };
        // The spec's "sos" is treated as a strong start regardless of
        // L/R — but for W2 we only care whether the most recent
        // strong is AL. sos is never AL (paragraph_level returns 0 or
        // 1, mapped to L or R by the X1 stack frame). So the initial
        // value being L or R is fine.
        for cls in classes.iter_mut() {
            match *cls {
                BidiClass::L | BidiClass::R | BidiClass::AL => last_strong = *cls,
                BidiClass::EN if last_strong == BidiClass::AL => {
                    *cls = BidiClass::AN;
                }
                _ => {}
            }
        }
    }

    // --- W3: every remaining AL becomes R -------------------------
    //
    // Trivial collapse; must run *after* W2 because W2 reads AL.
    for cls in classes.iter_mut() {
        if *cls == BidiClass::AL {
            *cls = BidiClass::R;
        }
    }

    // --- W4: single ES between two ENs → EN; single CS between -----
    //         two of the same number type → that type. -------------
    //
    // Per spec examples:
    //   EN ES EN → EN EN EN
    //   EN CS EN → EN EN EN
    //   AN CS AN → AN AN AN
    //
    // The rule is narrow: the separator must be a *single* character,
    // with the same number type on both sides. We do this in one
    // forward pass — for each position i where classes[i] is ES or
    // CS, look at i-1 and i+1.
    if classes.len() >= 3 {
        for i in 1..classes.len() - 1 {
            let cur = classes[i];
            let prev = classes[i - 1];
            let next = classes[i + 1];
            match cur {
                BidiClass::ES if prev == BidiClass::EN && next == BidiClass::EN => {
                    classes[i] = BidiClass::EN;
                }
                BidiClass::CS if prev == BidiClass::EN && next == BidiClass::EN => {
                    classes[i] = BidiClass::EN;
                }
                BidiClass::CS if prev == BidiClass::AN && next == BidiClass::AN => {
                    classes[i] = BidiClass::AN;
                }
                _ => {}
            }
        }
    }

    // --- W5: ET adjacent to EN (on either side) → EN --------------
    //
    // Per spec examples:
    //   ET ET EN → EN EN EN   (leading ETs adjacent via the trailing EN)
    //   EN ET ET → EN EN EN   (trailing ETs adjacent via the leading EN)
    //   AN ET EN → AN EN EN   (only the EN-adjacent side flips; the
    //                          ET adjacent to AN does NOT flip because
    //                          the rule says "adjacent to European
    //                          numbers", and AN is not EN).
    //
    // Strategy: find every contiguous run of ETs. The run flips to EN
    // iff it touches an EN on at least one side.
    {
        let n = classes.len();
        let mut i = 0;
        while i < n {
            if classes[i] != BidiClass::ET {
                i += 1;
                continue;
            }
            let start = i;
            while i < n && classes[i] == BidiClass::ET {
                i += 1;
            }
            let end = i; // exclusive
            let left_en = start > 0 && classes[start - 1] == BidiClass::EN;
            let right_en = end < n && classes[end] == BidiClass::EN;
            if left_en || right_en {
                for cls in &mut classes[start..end] {
                    *cls = BidiClass::EN;
                }
            }
        }
    }

    // --- W6: all remaining separators / terminators → ON ----------
    //
    // After W4 + W5, anything that is still ES / ET / CS is a
    // separator that did not get absorbed into a number. Per spec it
    // becomes Other Neutral.
    for cls in classes.iter_mut() {
        if matches!(*cls, BidiClass::ES | BidiClass::ET | BidiClass::CS) {
            *cls = BidiClass::ON;
        }
    }

    // --- W7: EN whose most-recent strong (L / R / sos) is L → L ---
    //
    // Note: W3 has already turned every AL into R, so the strong-type
    // backward walk for W7 only sees L / R / sos. Forward sweep with
    // the same "last strong" tracker as W2.
    {
        let mut last_strong = if matches!(sos, BidiClass::L | BidiClass::R) {
            sos
        } else {
            // sos must be L or R after X1's level-mapping; treat any
            // unexpected non-strong sos as L (the W7 effect is the
            // same as "no preceding strong yet").
            BidiClass::L
        };
        for cls in classes.iter_mut() {
            match *cls {
                BidiClass::L | BidiClass::R => last_strong = *cls,
                BidiClass::EN if last_strong == BidiClass::L => {
                    *cls = BidiClass::L;
                }
                _ => {}
            }
        }
    }
}

/// Resolve **neutral and isolate-formatting types** for one
/// isolating run sequence per UAX #9 **N1, N2** (§3.3.5).
///
/// N0 (bracket-pair resolution) is **not** applied by this routine
/// — it requires the Unicode `BidiBrackets.txt` data file to
/// identify opening / closing paired brackets, which is a follow-up
/// dependency. Callers that need N0 should run it *before* calling
/// this function so that any bracket-resolved positions are already
/// strong types by the time N1 walks them.
///
/// The input `classes` are the per-character [`BidiClass`] values
/// for **one isolating run sequence** in logical order — the same
/// slice already mutated by [`resolve_weak_types`]. The slice must
/// already be free of `AL` (collapsed to `R` by W3) and of leftover
/// `ES` / `ET` / `CS` (collapsed to `ON` by W6) — feeding the W
/// pass's output guarantees that. `embedding_level` is the
/// embedding level of the run as a whole (`0` for an LTR
/// paragraph's outer run, `1` for an RTL paragraph's outer run; the
/// X-stack drives this for nested runs). `sos` / `eos` are the
/// **start- and end-of-sequence strong types** (`L` or `R`,
/// derived from the X-stack frame for the run).
///
/// The function mutates `classes` in place. After return every
/// element is one of `L`, `R`, `EN`, `AN`, `NSM`, or `BN` — every
/// NI (`B` / `S` / `WS` / `ON` / `LRI` / `RLI` / `FSI` / `PDI`) has
/// been resolved to a strong direction by either N1 (matching
/// strong neighbours, with `EN` / `AN` counting as `R`) or N2
/// (embedding direction fallback when strong neighbours differ or
/// the sequence boundary is on the other side of an `NI`-only
/// run). `NSM` and `BN` are intentionally left alone — they are
/// not in the NI alias and the §3.3.6 implicit-level rules handle
/// them.
///
/// The implementation is a single forward sweep:
///
/// 1. Find every maximal contiguous run `[start, end)` of
///    `classes[i].is_neutral_or_isolate()` elements.
/// 2. Determine the **left strong** type: the previous
///    non-NI / non-NSM / non-BN element's "directional contribution"
///    (`L` stays `L`; `R` / `EN` / `AN` all count as `R` per the
///    spec's "European and Arabic numbers act as if they were R");
///    falls back to `sos`'s direction at the head of the sequence.
/// 3. Determine the **right strong** type symmetrically; falls back
///    to `eos`'s direction at the tail.
/// 4. If `left == right`, apply **N1** — rewrite every element of
///    the run to that strong type.
/// 5. Otherwise apply **N2** — rewrite every element of the run to
///    the embedding direction (`L` for even `embedding_level`, `R`
///    for odd).
///
/// Provenance: rules transcribed verbatim from
/// `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` §3.3.5 (UAX
/// #9 Revision 50, Unicode 16.0).
///
/// # Examples
///
/// ```
/// use oxideav_scribe::bidi::{resolve_neutral_types, BidiClass};
///
/// // Spec example "L NI L → L L L".
/// let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::L];
/// resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
/// assert_eq!(cls, vec![BidiClass::L, BidiClass::L, BidiClass::L]);
///
/// // Spec example "R NI AN → R R AN" (AN counts as R for N1).
/// let mut cls = vec![BidiClass::R, BidiClass::ON, BidiClass::AN];
/// resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
/// assert_eq!(cls, vec![BidiClass::R, BidiClass::R, BidiClass::AN]);
///
/// // N2 fallback: differing-strong-context NIs take the embedding
/// // direction. With embedding_level 0 (L), the unresolved NI
/// // between L and R becomes L.
/// let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::R];
/// resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::R);
/// assert_eq!(cls, vec![BidiClass::L, BidiClass::L, BidiClass::R]);
/// ```
pub fn resolve_neutral_types(
    classes: &mut [BidiClass],
    embedding_level: u8,
    sos: BidiClass,
    eos: BidiClass,
) {
    if classes.is_empty() {
        return;
    }

    // N0..N2 narration: "European and Arabic numbers act as if they
    // were R in terms of their influence on NIs." So the strong-type
    // search treats EN / AN as R. Helper to project a Bidi class onto
    // its strong-direction contribution.
    fn strong_dir(c: BidiClass) -> Option<BidiClass> {
        match c {
            BidiClass::L => Some(BidiClass::L),
            BidiClass::R | BidiClass::EN | BidiClass::AN => Some(BidiClass::R),
            _ => None,
        }
    }

    // sos / eos are strong types (L or R after X-stack mapping); the
    // function tolerates any input but maps non-strong sos/eos to L
    // for safety (consistent with W2 / W7).
    let sos_dir = strong_dir(sos).unwrap_or(BidiClass::L);
    let eos_dir = strong_dir(eos).unwrap_or(BidiClass::L);

    let embedding_dir = if embedding_level % 2 == 0 {
        BidiClass::L
    } else {
        BidiClass::R
    };

    let n = classes.len();
    let mut i = 0;
    while i < n {
        if !classes[i].is_neutral_or_isolate() {
            i += 1;
            continue;
        }
        // Found the start of an NI run.
        let start = i;
        while i < n && classes[i].is_neutral_or_isolate() {
            i += 1;
        }
        let end = i; // exclusive

        // Left strong: walk backward from `start - 1` until we find a
        // strong-direction contributor (L / R / EN / AN). Skip NSM /
        // BN, which are non-strong but not NI either. If we hit the
        // sequence head, fall back to sos_dir.
        let mut left = sos_dir;
        if start > 0 {
            let mut k = start;
            while k > 0 {
                k -= 1;
                if let Some(d) = strong_dir(classes[k]) {
                    left = d;
                    break;
                }
                if k == 0 {
                    // walked off the head without finding a strong
                    // contributor — fall back to sos_dir (already in
                    // `left`).
                    break;
                }
            }
        }

        // Right strong: walk forward from `end` until we find a
        // strong-direction contributor. If we hit the sequence tail,
        // fall back to eos_dir.
        let mut right = eos_dir;
        {
            let mut k = end;
            while k < n {
                if let Some(d) = strong_dir(classes[k]) {
                    right = d;
                    break;
                }
                k += 1;
            }
        }

        let target = if left == right { left } else { embedding_dir };
        for cls in &mut classes[start..end] {
            *cls = target;
        }
    }
}

/// Resolve **implicit embedding levels** for one isolating run sequence
/// per UAX #9 **I1, I2** (§3.3.6).
///
/// `classes` is the per-character [`BidiClass`] vector left by the N
/// pass — every former neutral or isolate-formatting position has
/// already been collapsed to a strong type, and the only weak types
/// that survive are `EN`, `AN`, `NSM`, and `BN` (per UAX #9 §3.3.4 +
/// §3.3.5). `embedding_level` is the embedding level of the run as a
/// whole (`0` for an LTR paragraph's outer run, `1` for an RTL
/// paragraph's outer run; the X-stack drives this for nested runs).
///
/// Returns a `Vec<u8>` of the same length as `classes`, holding the
/// per-character **resolved** embedding level. Per UAX #9 §3.3.6
/// Table 5:
///
/// | Type | Even EL | Odd EL |
/// | ---- | ------- | ------ |
/// | L    | EL      | EL+1   |
/// | R    | EL+1    | EL     |
/// | AN   | EL+2    | EL+1   |
/// | EN   | EL+2    | EL+1   |
///
/// `BN` is ignored per UAX #9 §5.2 ("In rules I1 and I2, ignore BN.")
/// — its level stays at `embedding_level`, so a later L1 / L4 pass
/// (which resets BN levels in a separate phase) sees a stable base.
/// `NSM` was rewritten to its preceding character's type by W1; if
/// it survived the N pass unchanged (only the explicit
/// `BidiClass::NSM` slot does, after the N-rules pass), it is also
/// treated as `BN`-like here — its level stays at `embedding_level`
/// because it is not a strong / numeric type and the spec's I1 / I2
/// rules enumerate only L / R / AN / EN.
///
/// # Examples
///
/// ```
/// use oxideav_scribe::bidi::{resolve_implicit_levels, BidiClass};
///
/// // Even (LTR) paragraph: L stays, R goes +1, EN / AN go +2.
/// let cls = vec![BidiClass::L, BidiClass::R, BidiClass::EN, BidiClass::AN];
/// assert_eq!(resolve_implicit_levels(&cls, 0), vec![0, 1, 2, 2]);
///
/// // Odd (RTL) paragraph: R stays, L / EN / AN all go +1.
/// let cls = vec![BidiClass::L, BidiClass::R, BidiClass::EN, BidiClass::AN];
/// assert_eq!(resolve_implicit_levels(&cls, 1), vec![2, 1, 2, 2]);
/// ```
#[must_use]
pub fn resolve_implicit_levels(classes: &[BidiClass], embedding_level: u8) -> Vec<u8> {
    let even = embedding_level % 2 == 0;
    classes
        .iter()
        .map(|c| match c {
            BidiClass::L => {
                if even {
                    embedding_level
                } else {
                    embedding_level + 1
                }
            }
            BidiClass::R => {
                if even {
                    embedding_level + 1
                } else {
                    embedding_level
                }
            }
            BidiClass::EN | BidiClass::AN => {
                if even {
                    embedding_level + 2
                } else {
                    embedding_level + 1
                }
            }
            // §5.2: "In rules I1 and I2, ignore BN." A surviving NSM
            // is similarly outside the I1 / I2 enumeration (the spec
            // only names L / R / AN / EN); leave it at the embedding
            // level so a follow-up L-rule pass can fold it.
            _ => embedding_level,
        })
        .collect()
}

/// Apply UAX #9 §3.4 rule **L1** to one line in place.
///
/// L1 resets the embedding level of certain trailing / separator
/// characters back to the paragraph embedding level so that
/// whitespace and tabulation end up on the visual edge that
/// matches the paragraph direction. The four sub-cases enumerated
/// in §3.4 are:
///
/// 1. Segment separators (class `S`).
/// 2. Paragraph separators (class `B`).
/// 3. Any sequence of whitespace (`WS`) and/or isolate-formatting
///    characters (`LRI` / `RLI` / `FSI` / `PDI`) **preceding** a
///    segment separator or paragraph separator.
/// 4. Any sequence of whitespace and/or isolate-formatting
///    characters **at the end of the line**.
///
/// UAX #9 §3.4 carries a normative note: "The types of characters
/// used here are the *original* types, not those modified by the
/// previous phase." `orig_classes` is therefore the same class
/// slice the caller fed into `resolve_weak_types` / the N-rule /
/// I-rule passes — not the post-W/N output.
///
/// `levels` is the per-character level vector produced by
/// [`resolve_implicit_levels`] (the §3.3.6 output) for the
/// characters that make up this one line. The function rewrites
/// the affected positions of `levels` in place to
/// `paragraph_level`; positions that L1 does not name (strong
/// characters, weak numerics, leftover neutrals that are not on a
/// trailing-whitespace run) are left untouched.
///
/// # Panics
///
/// Panics if `orig_classes.len() != levels.len()`.
///
/// # Examples
///
/// ```
/// use oxideav_scribe::bidi::{reset_trailing_levels, BidiClass};
///
/// // Trailing space in an LTR paragraph stays at level 0.
/// // (Resolved levels from a prior I pass might be 0 for the `L`
/// // text and the trailing `WS`; L1 explicitly anchors the WS to
/// // the paragraph level either way.)
/// let cls = vec![BidiClass::L, BidiClass::L, BidiClass::WS];
/// let mut lvl = vec![0, 0, 0];
/// reset_trailing_levels(&cls, &mut lvl, 0);
/// assert_eq!(lvl, vec![0, 0, 0]);
///
/// // RTL paragraph: trailing whitespace is dragged to level 1.
/// let cls = vec![BidiClass::R, BidiClass::R, BidiClass::WS, BidiClass::WS];
/// let mut lvl = vec![1, 1, 2, 2];
/// reset_trailing_levels(&cls, &mut lvl, 1);
/// assert_eq!(lvl, vec![1, 1, 1, 1]);
/// ```
pub fn reset_trailing_levels(orig_classes: &[BidiClass], levels: &mut [u8], paragraph_level: u8) {
    assert_eq!(
        orig_classes.len(),
        levels.len(),
        "reset_trailing_levels: class slice and level slice must be the same length",
    );
    let n = orig_classes.len();
    if n == 0 {
        return;
    }
    // Cases (1) + (2): every S / B position is reset directly.
    // Case (3): for each such separator, walk leftward across any
    // contiguous WS / isolate-formatting run and reset those too.
    // Case (4): a single trailing WS / isolate-formatting run at
    // the end of the line is reset.
    for (i, &cls) in orig_classes.iter().enumerate() {
        if matches!(cls, BidiClass::S | BidiClass::B) {
            levels[i] = paragraph_level;
            // Walk backward over WS + isolate-formatting characters
            // immediately preceding this separator.
            let mut j = i;
            while j > 0 && is_l1_trailing_filler(orig_classes[j - 1]) {
                j -= 1;
                levels[j] = paragraph_level;
            }
        }
    }
    // Case (4): trailing WS + isolate-formatting at end of line.
    let mut k = n;
    while k > 0 && is_l1_trailing_filler(orig_classes[k - 1]) {
        k -= 1;
        levels[k] = paragraph_level;
    }
}

/// Predicate for the §3.4 L1 case-(3) / case-(4) "whitespace +
/// isolate-formatting" set: `WS`, `LRI`, `RLI`, `FSI`, `PDI`.
fn is_l1_trailing_filler(c: BidiClass) -> bool {
    matches!(
        c,
        BidiClass::WS | BidiClass::LRI | BidiClass::RLI | BidiClass::FSI | BidiClass::PDI
    )
}

/// Apply UAX #9 §3.4 rule **L2** to one line and return a logical-
/// to-visual permutation.
///
/// The returned `Vec<usize>` has `levels.len()` entries; entry `v`
/// is the logical index that should be displayed at visual
/// position `v`. The caller (a renderer / line builder) walks the
/// permutation in order and emits the glyphs of the corresponding
/// logical characters left-to-right.
///
/// The algorithm is the spec's progressive-reversal procedure:
///
/// 1. Start with the identity permutation `[0, 1, ..., n - 1]`.
/// 2. Find `max_level` (the largest entry of `levels`).
/// 3. Find `lowest_odd_level` (the smallest odd entry of `levels`;
///    if no odd level exists the line is wholly LTR and no
///    reversal is needed).
/// 4. For each iteration level `L = max_level, max_level - 1, ...,
///    lowest_odd_level`, find every maximal contiguous run of
///    positions whose original (pre-L1, but the §3.4 algorithm
///    operates on the post-L1 vector here) level is `>= L`, and
///    reverse the permutation entries in that range.
///
/// The progressive scan from the top down builds up the nested
/// reversals shown in UAX #9 §3.4 Examples 1..4: a level-1 run
/// inside a level-0 paragraph reverses once; a level-2 number
/// embedded in a level-1 RTL run reverses once at level 2 (the
/// digits go LTR within the embedding) and then again at level 1
/// (the whole embedding goes RTL within the paragraph).
///
/// Returns the identity permutation when `levels` is empty.
///
/// # Examples
///
/// ```
/// use oxideav_scribe::bidi::reorder_line;
///
/// // All-LTR line: identity.
/// assert_eq!(reorder_line(&[0, 0, 0]), vec![0, 1, 2]);
///
/// // All-RTL line: full reverse.
/// assert_eq!(reorder_line(&[1, 1, 1]), vec![2, 1, 0]);
///
/// // §3.4 Example 1: "car means CAR.", resolved levels
/// // 00000000001110 — only the level-1 run "CAR" reverses.
/// let lv = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0];
/// let visual = reorder_line(&lv);
/// // Positions 10..13 reverse to 12, 11, 10; trailing '.' stays.
/// assert_eq!(visual, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 11, 10, 13]);
/// ```
#[must_use]
pub fn reorder_line(levels: &[u8]) -> Vec<usize> {
    let n = levels.len();
    let mut visual: Vec<usize> = (0..n).collect();
    if n == 0 {
        return visual;
    }
    let max_level = *levels.iter().max().unwrap_or(&0);
    let lowest_odd_level = levels
        .iter()
        .copied()
        .filter(|l| l % 2 == 1)
        .min()
        .unwrap_or(u8::MAX);
    if lowest_odd_level == u8::MAX {
        // No odd levels: the whole line is LTR. L2's lower bound is
        // the lowest odd level, so no iteration runs.
        return visual;
    }
    // For each level from max down to lowest_odd_level, reverse
    // every maximal contiguous run of positions whose level is
    // `>= level`.
    let mut level = max_level;
    loop {
        let mut i = 0;
        while i < n {
            if levels[i] >= level {
                let mut j = i + 1;
                while j < n && levels[j] >= level {
                    j += 1;
                }
                visual[i..j].reverse();
                i = j;
            } else {
                i += 1;
            }
        }
        if level == lowest_odd_level {
            break;
        }
        // level >= lowest_odd_level >= 1, so the decrement is safe.
        level -= 1;
    }
    visual
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

    // --- Section 6: NI predicate -------------------------------

    #[test]
    fn neutral_or_isolate_predicate_covers_uax9_ni_alias() {
        // NI alias = neutrals (B/S/WS/ON) ∪ isolate-formatting
        // (FSI/LRI/RLI/PDI). Every member tests true.
        for c in [
            BidiClass::B,
            BidiClass::S,
            BidiClass::WS,
            BidiClass::ON,
            BidiClass::FSI,
            BidiClass::LRI,
            BidiClass::RLI,
            BidiClass::PDI,
        ] {
            assert!(c.is_neutral_or_isolate(), "{c:?} should be NI");
        }
        // Strong / weak / embedding-formatting / PDF are NOT NI.
        for c in [
            BidiClass::L,
            BidiClass::R,
            BidiClass::AL,
            BidiClass::EN,
            BidiClass::ES,
            BidiClass::ET,
            BidiClass::AN,
            BidiClass::CS,
            BidiClass::NSM,
            BidiClass::BN,
            BidiClass::LRE,
            BidiClass::LRO,
            BidiClass::RLE,
            BidiClass::RLO,
            BidiClass::PDF,
        ] {
            assert!(!c.is_neutral_or_isolate(), "{c:?} should not be NI");
        }
    }

    // --- Section 7: W rules (W1..W7) ---------------------------

    #[test]
    fn w_rules_empty_input_is_noop() {
        let mut cls: Vec<BidiClass> = vec![];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert!(cls.is_empty());
    }

    #[test]
    fn w1_consecutive_nsm_inherit_first_strongs_type() {
        // Spec example: AL NSM NSM → AL AL AL (forward pass; second NSM
        // sees the first NSM after rewrite).
        let mut cls = vec![BidiClass::AL, BidiClass::NSM, BidiClass::NSM];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        // After W1: AL AL AL. After W3: R R R.
        assert_eq!(cls, vec![BidiClass::R, BidiClass::R, BidiClass::R]);
    }

    #[test]
    fn w1_nsm_at_sequence_start_takes_sos_type() {
        // Spec example: <sos=R> NSM → <sos> R. Then W3 has no AL to
        // collapse, so the NSM stays R.
        let mut cls = vec![BidiClass::NSM, BidiClass::L];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::L]);
    }

    #[test]
    fn w1_nsm_after_isolate_initiator_or_pdi_becomes_on() {
        // Spec example: LRI NSM → LRI ON; PDI NSM → PDI ON.
        let mut cls = vec![BidiClass::LRI, BidiClass::NSM];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::LRI, BidiClass::ON]);

        let mut cls = vec![BidiClass::PDI, BidiClass::NSM];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::PDI, BidiClass::ON]);

        let mut cls = vec![BidiClass::RLI, BidiClass::NSM];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::RLI, BidiClass::ON]);

        let mut cls = vec![BidiClass::FSI, BidiClass::NSM];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::FSI, BidiClass::ON]);
    }

    #[test]
    fn w2_en_after_al_strong_becomes_an() {
        // Spec example: AL EN → AL AN. After W3 the AL collapses to R.
        let mut cls = vec![BidiClass::AL, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::AN]);
        // AL NI EN → AL NI AN (the NI is ON which doesn't touch the
        // last-strong tracker).
        let mut cls = vec![BidiClass::AL, BidiClass::ON, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::ON, BidiClass::AN]);
    }

    #[test]
    fn w2_en_with_no_al_predecessor_stays_en() {
        // sos=L, no AL → EN stays EN. (W7 may yet flip it to L; see
        // dedicated test.)
        let mut cls = vec![BidiClass::ON, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        // sos=L → W7 fires: last_strong is L, EN → L.
        assert_eq!(cls, vec![BidiClass::ON, BidiClass::L]);
        // L NI EN → L NI EN (W2: last strong is L, not AL); after W7
        // the EN becomes L.
        let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::L, BidiClass::ON, BidiClass::L]);
        // R NI EN → R NI EN: W7 sees R as last strong, leaves EN alone.
        let mut cls = vec![BidiClass::R, BidiClass::ON, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::ON, BidiClass::EN]);
    }

    #[test]
    fn w2_sos_alone_does_not_flip_en() {
        // sos NI EN → sos NI EN (W2: sos is not AL).
        // With sos=L, W7 then fires → EN becomes L.
        let mut cls = vec![BidiClass::ON, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::ON, BidiClass::L]);
        // With sos=R, W7 does not fire → EN stays.
        let mut cls = vec![BidiClass::ON, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::ON, BidiClass::EN]);
    }

    #[test]
    fn w3_all_remaining_al_become_r() {
        // Pure AL run → R run.
        let mut cls = vec![BidiClass::AL, BidiClass::AL, BidiClass::AL];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::R, BidiClass::R]);
    }

    #[test]
    fn w4_single_es_or_cs_between_two_ens_collapses_to_en() {
        // Spec: EN ES EN → EN EN EN.
        let mut cls = vec![BidiClass::EN, BidiClass::ES, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::EN, BidiClass::EN, BidiClass::EN]);
        // Spec: EN CS EN → EN EN EN.
        let mut cls = vec![BidiClass::EN, BidiClass::CS, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::EN, BidiClass::EN, BidiClass::EN]);
        // Spec: AN CS AN → AN AN AN (CS between same-type AN both
        // sides flips).
        let mut cls = vec![BidiClass::AN, BidiClass::CS, BidiClass::AN];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::AN, BidiClass::AN, BidiClass::AN]);
    }

    #[test]
    fn w4_does_not_collapse_mixed_or_multiple_separators() {
        // Mixed-type CS (EN CS AN) does NOT collapse (W4 demands same
        // type both sides).
        let mut cls = vec![BidiClass::EN, BidiClass::CS, BidiClass::AN];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        // CS doesn't match W4, W6 turns it into ON.
        assert_eq!(cls, vec![BidiClass::EN, BidiClass::ON, BidiClass::AN]);
        // Two consecutive ES are NOT a "single ES" — neither flips.
        let mut cls = vec![BidiClass::EN, BidiClass::ES, BidiClass::ES, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(
            cls,
            vec![BidiClass::EN, BidiClass::ON, BidiClass::ON, BidiClass::EN]
        );
        // AN ES AN does NOT collapse — W4 covers CS only for AN, not ES.
        let mut cls = vec![BidiClass::AN, BidiClass::ES, BidiClass::AN];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::AN, BidiClass::ON, BidiClass::AN]);
    }

    #[test]
    fn w5_ets_adjacent_to_en_collapse() {
        // Spec: ET ET EN → EN EN EN.
        let mut cls = vec![BidiClass::ET, BidiClass::ET, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::EN, BidiClass::EN, BidiClass::EN]);
        // Spec: EN ET ET → EN EN EN.
        let mut cls = vec![BidiClass::EN, BidiClass::ET, BidiClass::ET];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::EN, BidiClass::EN, BidiClass::EN]);
        // Spec: AN ET EN → AN EN EN (the ET is adjacent to EN on the
        // right side, so it flips; the AN on the left does not push
        // anything because AN is not EN).
        let mut cls = vec![BidiClass::AN, BidiClass::ET, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::AN, BidiClass::EN, BidiClass::EN]);
    }

    #[test]
    fn w5_isolated_ets_do_not_collapse() {
        // A solitary ET with no EN neighbour stays ET → W6 → ON.
        let mut cls = vec![BidiClass::R, BidiClass::ET, BidiClass::R];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::ON, BidiClass::R]);
        // ET-only run far from any EN → ON ON.
        let mut cls = vec![BidiClass::ET, BidiClass::ET];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::ON, BidiClass::ON]);
    }

    #[test]
    fn w6_remaining_separators_terminators_become_on() {
        // Spec: AN ET → AN ON. (ET adjacent to AN does NOT flip; W5
        // is EN-only.)
        let mut cls = vec![BidiClass::AN, BidiClass::ET];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::AN, BidiClass::ON]);
        // Spec: L ES EN → L ON EN. ES has no EN on the left, so W4
        // doesn't fire; W6 turns it into ON. Then W7 sees L as last
        // strong → EN becomes L. Final: L ON L.
        let mut cls = vec![BidiClass::L, BidiClass::ES, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::L, BidiClass::ON, BidiClass::L]);
        // Spec: EN CS AN → EN ON AN.
        let mut cls = vec![BidiClass::EN, BidiClass::CS, BidiClass::AN];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::EN, BidiClass::ON, BidiClass::AN]);
        // Spec: ET AN → ON AN.
        let mut cls = vec![BidiClass::ET, BidiClass::AN];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::ON, BidiClass::AN]);
    }

    #[test]
    fn w7_en_after_l_becomes_l() {
        // Spec: L NI EN → L NI L.
        let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::L, BidiClass::ON, BidiClass::L]);
        // Spec: R NI EN → R NI EN (R as last strong leaves EN alone).
        let mut cls = vec![BidiClass::R, BidiClass::ON, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::ON, BidiClass::EN]);
    }

    #[test]
    fn w7_with_sos_l_flips_lone_en() {
        // sos=L, no L in the sequence, EN at end → W7 sees sos as L
        // and flips EN → L.
        let mut cls = vec![BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::L]);
        // sos=R, no L in the sequence → EN stays EN.
        let mut cls = vec![BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::EN]);
    }

    #[test]
    fn w_rules_compose_w2_before_w3_before_w7() {
        // Critical ordering check: AL EN → after W2 → AL AN → after W3
        // → R AN. W7 sees R as last strong (not L), so the AN is NOT
        // re-flipped (and W7 only inspects EN anyway). Confirms W2
        // fires *before* W3 (otherwise we would lose the AL marker
        // and EN would never flip to AN).
        let mut cls = vec![BidiClass::AL, BidiClass::EN, BidiClass::EN];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::AN, BidiClass::AN]);
    }

    // --- Section 8: N rules (N1 + N2) -------------------------

    #[test]
    fn n_rules_empty_input_is_noop() {
        let mut cls: Vec<BidiClass> = vec![];
        resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
        assert!(cls.is_empty());
    }

    #[test]
    fn n1_l_ni_l_collapses_to_l() {
        // Spec example: L NI L → L L L.
        let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::L];
        resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::L, BidiClass::L, BidiClass::L]);
    }

    #[test]
    fn n1_r_ni_r_collapses_to_r() {
        // Spec example: R NI R → R R R.
        let mut cls = vec![BidiClass::R, BidiClass::ON, BidiClass::R];
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::R, BidiClass::R]);
    }

    #[test]
    fn n1_numbers_count_as_r_for_surrounding_check() {
        // Spec table — exhaustive R/AN/EN cross-product.
        // R NI AN → R R AN.
        let mut cls = vec![BidiClass::R, BidiClass::ON, BidiClass::AN];
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::R, BidiClass::AN]);
        // R NI EN → R R EN.
        let mut cls = vec![BidiClass::R, BidiClass::ON, BidiClass::EN];
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::R, BidiClass::EN]);
        // AN NI R → AN R R.
        let mut cls = vec![BidiClass::AN, BidiClass::ON, BidiClass::R];
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::AN, BidiClass::R, BidiClass::R]);
        // AN NI AN → AN R AN.
        let mut cls = vec![BidiClass::AN, BidiClass::ON, BidiClass::AN];
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::AN, BidiClass::R, BidiClass::AN]);
        // AN NI EN → AN R EN.
        let mut cls = vec![BidiClass::AN, BidiClass::ON, BidiClass::EN];
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::AN, BidiClass::R, BidiClass::EN]);
        // EN NI R → EN R R.
        let mut cls = vec![BidiClass::EN, BidiClass::ON, BidiClass::R];
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::EN, BidiClass::R, BidiClass::R]);
        // EN NI AN → EN R AN.
        let mut cls = vec![BidiClass::EN, BidiClass::ON, BidiClass::AN];
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::EN, BidiClass::R, BidiClass::AN]);
        // EN NI EN → EN R EN.
        let mut cls = vec![BidiClass::EN, BidiClass::ON, BidiClass::EN];
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::EN, BidiClass::R, BidiClass::EN]);
    }

    #[test]
    fn n2_differing_strong_context_takes_embedding_direction() {
        // Spec example footnote: with eos=L sos=R the run "R NI eos"
        // resolves NI → e (the embedding direction). Here we shape
        // the same with explicit slices.
        //
        // L NI R, embedding_level 0 → L stays, NI takes embedding
        // direction L, R stays. Final: L L R.
        let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::R];
        resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::L, BidiClass::L, BidiClass::R]);
        // Same input with embedding_level 1 → NI takes R.
        let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::R];
        resolve_neutral_types(&mut cls, 1, BidiClass::L, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::L, BidiClass::R, BidiClass::R]);
        // R NI L mirror, embedding_level 1 → NI takes R.
        let mut cls = vec![BidiClass::R, BidiClass::ON, BidiClass::L];
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::R, BidiClass::L]);
    }

    #[test]
    fn n_rules_sos_eos_drive_boundary_runs() {
        // Spec example footnote: <sos=R> NI L → <sos> R L
        // (N1 sees R on left via sos, L on right; mismatch → N2 takes
        // embedding direction; here embedding 1 (R) → NI becomes R).
        let mut cls = vec![BidiClass::ON, BidiClass::L];
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::L);
        // Mismatch — embedding (1 = R) wins.
        assert_eq!(cls, vec![BidiClass::R, BidiClass::L]);
        // Same with embedding 0 (L): NI becomes L.
        let mut cls = vec![BidiClass::ON, BidiClass::L];
        resolve_neutral_types(&mut cls, 0, BidiClass::R, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::L, BidiClass::L]);
        // <sos=L> NI <eos=L>: both sides agree → N1 folds to L.
        let mut cls = vec![BidiClass::ON, BidiClass::WS];
        resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::L, BidiClass::L]);
        // <sos=R> NI <eos=R>: both sides agree → N1 folds to R.
        let mut cls = vec![BidiClass::ON, BidiClass::WS];
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::R]);
    }

    #[test]
    fn n_rules_long_ni_run_collapses_uniformly() {
        // A run of many NIs of mixed types (B / S / WS / ON / LRI /
        // RLI / FSI / PDI) all flip to the resolved direction in one
        // pass.
        let mut cls = vec![
            BidiClass::L,
            BidiClass::WS,
            BidiClass::ON,
            BidiClass::LRI,
            BidiClass::PDI,
            BidiClass::S,
            BidiClass::B,
            BidiClass::L,
        ];
        resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
        assert_eq!(
            cls,
            vec![
                BidiClass::L,
                BidiClass::L,
                BidiClass::L,
                BidiClass::L,
                BidiClass::L,
                BidiClass::L,
                BidiClass::L,
                BidiClass::L,
            ]
        );
    }

    #[test]
    fn n_rules_leave_nsm_and_bn_alone() {
        // NSM and BN are NOT in the NI alias (only the four neutrals
        // + four isolate-formatting types are). They must pass
        // through unchanged.
        let mut cls = vec![BidiClass::L, BidiClass::NSM, BidiClass::BN, BidiClass::L];
        resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
        assert_eq!(
            cls,
            vec![BidiClass::L, BidiClass::NSM, BidiClass::BN, BidiClass::L,]
        );
    }

    #[test]
    fn n_rules_nsm_does_not_terminate_ni_run() {
        // An NSM embedded in an NI run participates as a "skip" for
        // the strong-search — it is non-strong and non-NI, so the
        // strong-search walks past it. With L on both sides (one of
        // them past an NSM) the whole NI run still resolves to L via
        // N1.
        //
        // Layout: [L, NSM, ON, ON, L] — the NI run is positions 2..4.
        // Left strong: walk back from position 2 → see ON? no
        // (position 1 is NSM, position 0 is L). Wait — N1's "strong
        // type on either side" only considers strong-direction
        // contributors (L / R / EN / AN). NSM is neither. The walk
        // skips over it: left strong is L. Right strong is L. → run
        // becomes L.
        let mut cls = vec![
            BidiClass::L,
            BidiClass::NSM,
            BidiClass::ON,
            BidiClass::ON,
            BidiClass::L,
        ];
        resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
        assert_eq!(
            cls,
            vec![
                BidiClass::L,
                BidiClass::NSM,
                BidiClass::L,
                BidiClass::L,
                BidiClass::L,
            ]
        );
    }

    #[test]
    fn n_rules_multiple_independent_ni_runs() {
        // Two NI runs separated by an L. Each run resolves
        // independently against its own neighbours.
        // Layout: [R, ON, R, ON, ON, L]:
        //   Run 1 = [1..2], left=R right=R → R.
        //   Run 2 = [3..5], left=R right=L → mismatch → embedding (0
        //   = L).
        let mut cls = vec![
            BidiClass::R,
            BidiClass::ON,
            BidiClass::R,
            BidiClass::ON,
            BidiClass::ON,
            BidiClass::L,
        ];
        resolve_neutral_types(&mut cls, 0, BidiClass::R, BidiClass::L);
        assert_eq!(
            cls,
            vec![
                BidiClass::R,
                BidiClass::R,
                BidiClass::R,
                BidiClass::L,
                BidiClass::L,
                BidiClass::L,
            ]
        );
    }

    #[test]
    fn n_rules_ni_only_sequence_uses_sos_eos() {
        // No strong elements anywhere — both endpoints fall back to
        // sos / eos. With sos=L eos=L → both agree on L → run → L.
        let mut cls = vec![BidiClass::ON, BidiClass::WS, BidiClass::ON];
        resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
        assert_eq!(cls, vec![BidiClass::L, BidiClass::L, BidiClass::L]);
        // sos=L eos=R → mismatch → embedding (1 = R) → run → R.
        let mut cls = vec![BidiClass::ON, BidiClass::WS, BidiClass::ON];
        resolve_neutral_types(&mut cls, 1, BidiClass::L, BidiClass::R);
        assert_eq!(cls, vec![BidiClass::R, BidiClass::R, BidiClass::R]);
    }

    #[test]
    fn n_rules_compose_with_w_rules_realistic_run() {
        // Realistic full pipeline: start from a paragraph "AL NSM EN
        // ET EN CS AN" already used in the W7 composition test, push
        // it through both W and N. After W rules: [R R AN ON AN AN
        // AN]. Then N: position 3 is ON (the only NI), surrounded by
        // AN on both sides (which count as R) → N1 fires → AN
        // becomes R. Wait — AN is *not* an NI, and N1 *rewrites* the
        // NI itself. Position 3 is the ON; its neighbours are AN-3
        // (left: position 2) and AN-4 (right: position 4). AN counts
        // as R for the N1 search. left=R, right=R → ON → R.
        // Final: [R R AN R AN AN AN].
        let mut cls = vec![
            BidiClass::AL,
            BidiClass::NSM,
            BidiClass::EN,
            BidiClass::ET,
            BidiClass::EN,
            BidiClass::CS,
            BidiClass::AN,
        ];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
        assert_eq!(
            cls,
            vec![
                BidiClass::R,
                BidiClass::R,
                BidiClass::AN,
                BidiClass::R,
                BidiClass::AN,
                BidiClass::AN,
                BidiClass::AN,
            ]
        );
    }

    // --- Section 6: I1 / I2 implicit-level resolution -----------

    #[test]
    fn i1_even_level_l_stays_r_goes_up_one() {
        // Table 5 row 1 + 2 at even EL: L → EL, R → EL+1.
        let cls = vec![BidiClass::L, BidiClass::R, BidiClass::L, BidiClass::R];
        assert_eq!(resolve_implicit_levels(&cls, 0), vec![0, 1, 0, 1]);
        // Same shape at EL = 2.
        assert_eq!(resolve_implicit_levels(&cls, 2), vec![2, 3, 2, 3]);
    }

    #[test]
    fn i1_even_level_an_en_go_up_two() {
        // Table 5 row 3 + 4 at even EL: AN / EN → EL+2.
        let cls = vec![BidiClass::AN, BidiClass::EN, BidiClass::L, BidiClass::R];
        assert_eq!(resolve_implicit_levels(&cls, 0), vec![2, 2, 0, 1]);
        assert_eq!(resolve_implicit_levels(&cls, 2), vec![4, 4, 2, 3]);
    }

    #[test]
    fn i2_odd_level_l_en_an_go_up_one_r_stays() {
        // Table 5 odd column: L / EN / AN → EL+1; R → EL.
        let cls = vec![BidiClass::L, BidiClass::R, BidiClass::EN, BidiClass::AN];
        assert_eq!(resolve_implicit_levels(&cls, 1), vec![2, 1, 2, 2]);
        // Same shape at EL = 3.
        assert_eq!(resolve_implicit_levels(&cls, 3), vec![4, 3, 4, 4]);
    }

    #[test]
    fn implicit_levels_ignore_bn() {
        // §5.2 "In rules I1 and I2, ignore BN." A BN inserted between
        // L and R should sit at the embedding level, not bump.
        let cls = vec![BidiClass::L, BidiClass::BN, BidiClass::R];
        assert_eq!(resolve_implicit_levels(&cls, 0), vec![0, 0, 1]);
        assert_eq!(resolve_implicit_levels(&cls, 1), vec![2, 1, 1]);
    }

    #[test]
    fn implicit_levels_nsm_stays_at_embedding_level() {
        // NSM that survived the N pass (the rare case where W1 itself
        // left it as NSM — e.g. an NSM at the very start of a sequence
        // whose sos is also NSM-like / non-strong, the spec maps that
        // to ON via the §3.3.4 boundary rules and the N pass folds it,
        // but defensive behaviour matters here): keep it at the
        // embedding level, like BN.
        let cls = vec![BidiClass::L, BidiClass::NSM, BidiClass::R];
        assert_eq!(resolve_implicit_levels(&cls, 0), vec![0, 0, 1]);
    }

    #[test]
    fn implicit_levels_empty_input_yields_empty_output() {
        assert_eq!(resolve_implicit_levels(&[], 0), Vec::<u8>::new());
        assert_eq!(resolve_implicit_levels(&[], 1), Vec::<u8>::new());
    }

    #[test]
    fn implicit_levels_compose_after_n_rules() {
        // End-to-end: feed a slice through W → N → I and check the
        // final level vector. Logical: "L NI L" at EL 0. After N1
        // (matching L on both sides), all three are L → all sit at 0.
        let mut cls = vec![BidiClass::L, BidiClass::ON, BidiClass::L];
        resolve_weak_types(&mut cls, BidiClass::L, BidiClass::L);
        resolve_neutral_types(&mut cls, 0, BidiClass::L, BidiClass::L);
        let levels = resolve_implicit_levels(&cls, 0);
        assert_eq!(levels, vec![0, 0, 0]);
    }

    #[test]
    fn implicit_levels_arabic_with_numbers_realistic() {
        // Mixed Arabic + EN at paragraph level 1 (RTL paragraph):
        // start with [AL NSM EN ET EN CS AN] (same shape as the
        // w_rules_full_pipeline_realistic_run case), push it through
        // W + N + I, check that AN / EN positions all end at level 2
        // (one above the EL-1 base), while the R positions stay at 1.
        let mut cls = vec![
            BidiClass::AL,
            BidiClass::NSM,
            BidiClass::EN,
            BidiClass::ET,
            BidiClass::EN,
            BidiClass::CS,
            BidiClass::AN,
        ];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        resolve_neutral_types(&mut cls, 1, BidiClass::R, BidiClass::R);
        // After W + N: [R R AN R AN AN AN] (per the realistic-run
        // test above). At EL 1: R → 1, AN → 2.
        let levels = resolve_implicit_levels(&cls, 1);
        assert_eq!(levels, vec![1, 1, 2, 1, 2, 2, 2]);
    }

    #[test]
    fn implicit_levels_max_depth_overflow_is_explicit() {
        // The spec note "it is possible for text to end up at level
        // max_depth+1 as a result of this process." We don't clamp;
        // a caller passing EL near 125 (max_depth) can see levels
        // 126 or 127. Test that the arithmetic is straightforward
        // (no panic, no clamp). At EL 124 (even): L → 124, R → 125,
        // EN → 126.
        let cls = vec![BidiClass::L, BidiClass::R, BidiClass::EN];
        assert_eq!(resolve_implicit_levels(&cls, 124), vec![124, 125, 126]);
        // At EL 125 (odd): L → 126, R → 125, EN → 126.
        assert_eq!(resolve_implicit_levels(&cls, 125), vec![126, 125, 126]);
    }

    #[test]
    fn w_rules_full_pipeline_realistic_run() {
        // A mock isolating run sequence drawn from a hypothetical
        // mixed Arabic + number paragraph: AL NSM EN ET EN CS AN.
        // Walk through:
        //   W1: NSM after AL → AL.   → [AL AL EN ET EN CS AN]
        //   W2: EN after AL strong → AN. The second EN also sees AL
        //        as the most recent strong (the AN we just wrote
        //        doesn't change last_strong because AN is not strong).
        //                              → [AL AL AN ET AN CS AN]
        //   W3: ALs → R.              → [R  R  AN ET AN CS AN]
        //   W4: CS between two ANs flips → AN. ET is not eligible
        //       under W4. (After W2 the prev/next of CS are AN.)
        //                              → [R  R  AN ET AN AN AN]
        //   W5: ET is NOT adjacent to an EN on either side (the AN
        //       on both sides is AN, not EN), so it doesn't flip.
        //                              → [R  R  AN ET AN AN AN]
        //   W6: lingering ET → ON.    → [R  R  AN ON AN AN AN]
        //   W7: only inspects EN; no EN survives.
        let mut cls = vec![
            BidiClass::AL,
            BidiClass::NSM,
            BidiClass::EN,
            BidiClass::ET,
            BidiClass::EN,
            BidiClass::CS,
            BidiClass::AN,
        ];
        resolve_weak_types(&mut cls, BidiClass::R, BidiClass::R);
        assert_eq!(
            cls,
            vec![
                BidiClass::R,
                BidiClass::R,
                BidiClass::AN,
                BidiClass::ON,
                BidiClass::AN,
                BidiClass::AN,
                BidiClass::AN,
            ]
        );
    }

    // --- Section 8: L-rule line-level transformations -----------

    #[test]
    fn l1_segment_separator_resets_to_paragraph_level() {
        // §3.4 case (1): an `S` (tab) inside an RTL paragraph
        // resets to paragraph level 1, not whatever the I-rule
        // pass left it at.
        let cls = vec![BidiClass::R, BidiClass::S, BidiClass::R];
        let mut lvl = vec![1, 2, 1];
        reset_trailing_levels(&cls, &mut lvl, 1);
        assert_eq!(lvl, vec![1, 1, 1]);
    }

    #[test]
    fn l1_paragraph_separator_resets_to_paragraph_level() {
        // §3.4 case (2): a `B` at the end of an RTL line resets
        // to paragraph level 1.
        let cls = vec![BidiClass::R, BidiClass::R, BidiClass::B];
        let mut lvl = vec![1, 1, 2];
        reset_trailing_levels(&cls, &mut lvl, 1);
        assert_eq!(lvl, vec![1, 1, 1]);
    }

    #[test]
    fn l1_whitespace_before_separator_resets() {
        // §3.4 case (3): a WS run immediately preceding the
        // separator is folded onto the paragraph level too.
        let cls = vec![
            BidiClass::R,
            BidiClass::R,
            BidiClass::WS,
            BidiClass::WS,
            BidiClass::S,
        ];
        let mut lvl = vec![1, 1, 2, 2, 2];
        reset_trailing_levels(&cls, &mut lvl, 1);
        assert_eq!(lvl, vec![1, 1, 1, 1, 1]);
    }

    #[test]
    fn l1_isolate_formatting_before_separator_resets() {
        // §3.4 case (3): isolate-formatting characters (LRI / RLI
        // / FSI / PDI) count alongside WS in the trailing-filler
        // set.
        let cls = vec![
            BidiClass::L,
            BidiClass::WS,
            BidiClass::PDI,
            BidiClass::LRI,
            BidiClass::B,
        ];
        let mut lvl = vec![0, 1, 1, 1, 1];
        reset_trailing_levels(&cls, &mut lvl, 0);
        assert_eq!(lvl, vec![0, 0, 0, 0, 0]);
    }

    #[test]
    fn l1_trailing_whitespace_at_end_of_line_resets() {
        // §3.4 case (4): no separator, but trailing WS still
        // resets to paragraph level.
        let cls = vec![BidiClass::R, BidiClass::R, BidiClass::WS, BidiClass::WS];
        let mut lvl = vec![1, 1, 2, 2];
        reset_trailing_levels(&cls, &mut lvl, 1);
        assert_eq!(lvl, vec![1, 1, 1, 1]);
    }

    #[test]
    fn l1_leading_whitespace_is_left_alone() {
        // §3.4 cases (3) + (4) target trailing fillers only;
        // leading WS (without a separator behind it) keeps its
        // I-rule level.
        let cls = vec![BidiClass::WS, BidiClass::WS, BidiClass::R, BidiClass::R];
        let mut lvl = vec![2, 2, 1, 1];
        reset_trailing_levels(&cls, &mut lvl, 1);
        assert_eq!(lvl, vec![2, 2, 1, 1]);
    }

    #[test]
    fn l1_interior_whitespace_is_left_alone() {
        // Whitespace surrounded by strong characters on both
        // sides is neither case (3) (no following separator) nor
        // case (4) (not at end of line). It keeps its I level.
        let cls = vec![BidiClass::R, BidiClass::WS, BidiClass::R];
        let mut lvl = vec![1, 2, 1];
        reset_trailing_levels(&cls, &mut lvl, 1);
        assert_eq!(lvl, vec![1, 2, 1]);
    }

    #[test]
    fn l1_empty_line_is_noop() {
        let cls: Vec<BidiClass> = Vec::new();
        let mut lvl: Vec<u8> = Vec::new();
        reset_trailing_levels(&cls, &mut lvl, 0);
        assert!(lvl.is_empty());
    }

    #[test]
    fn l1_uses_original_classes_not_post_w_rules() {
        // §3.4 normative note: "The types of characters used here
        // are the *original* types, not those modified by the
        // previous phase." Here the original is `B` (a paragraph
        // separator); a W-rule pass cannot reach `B`, but L1 sees
        // it directly through `orig_classes`.
        let cls_orig = vec![BidiClass::R, BidiClass::R, BidiClass::B];
        let mut lvl = vec![1, 1, 2];
        reset_trailing_levels(&cls_orig, &mut lvl, 1);
        assert_eq!(lvl, vec![1, 1, 1]);
    }

    #[test]
    fn l1_multiple_separators_each_pull_their_preceding_whitespace() {
        // Two `S`s on one line: each resets its preceding WS
        // independently.
        let cls = vec![
            BidiClass::R,
            BidiClass::WS,
            BidiClass::S,
            BidiClass::R,
            BidiClass::WS,
            BidiClass::S,
        ];
        let mut lvl = vec![1, 2, 2, 1, 2, 2];
        reset_trailing_levels(&cls, &mut lvl, 1);
        assert_eq!(lvl, vec![1, 1, 1, 1, 1, 1]);
    }

    #[test]
    #[should_panic(expected = "same length")]
    fn l1_length_mismatch_panics() {
        let cls = vec![BidiClass::L, BidiClass::L];
        let mut lvl = vec![0];
        reset_trailing_levels(&cls, &mut lvl, 0);
    }

    #[test]
    fn l2_all_ltr_is_identity() {
        assert_eq!(reorder_line(&[0, 0, 0, 0]), vec![0, 1, 2, 3]);
    }

    #[test]
    fn l2_empty_input_is_empty() {
        let out = reorder_line(&[]);
        assert!(out.is_empty());
    }

    #[test]
    fn l2_all_rtl_is_full_reverse() {
        // Whole line at level 1: a single reversal flips it.
        assert_eq!(reorder_line(&[1, 1, 1, 1]), vec![3, 2, 1, 0]);
    }

    #[test]
    fn l2_uax9_example_1_car_means_car_dot() {
        // §3.4 Example 1: "car means CAR." with resolved levels
        // 00000000001110. Only the level-1 run reverses; the
        // trailing '.' stays put.
        let lv = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0];
        let visual = reorder_line(&lv);
        assert_eq!(visual, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 11, 10, 13]);
    }

    #[test]
    fn l2_uax9_example_2_nested_level_1_and_2() {
        // §3.4 Example 2: "<car MEANS CAR.=" resolved levels
        // 0222111111111110 (16 chars). Pass at level 2 reverses
        // the "rac" run (positions 1..4). Pass at level 1 reverses
        // positions 1..15.
        let lv = [0, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0];
        let visual = reorder_line(&lv);
        // After level-2 pass: identity except [1, 2, 3] -> [3, 2, 1].
        // After level-1 pass: positions 1..15 reverse, so the
        // final visual order is:
        //   0, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 1, 2, 3, 15
        assert_eq!(
            visual,
            vec![0, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 1, 2, 3, 15]
        );
    }

    #[test]
    fn l2_uax9_example_4_rtl_paragraph_deep_nesting() {
        // §3.4 Example 4 (embedding level = 1) resolved levels
        // 111111111111114222222222444333333333322111 — 42 chars.
        let lv = [
            1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 0..13
            2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // 14..23
            4, 4, 4, // 24..26
            3, 3, 3, 3, 3, 3, 3, 3, 3, 3, // 27..36
            2, 2, // 37..38
            1, 1, 1, // 39..41
        ];
        // Reproduce the spec's display by stepping the algorithm
        // by hand:
        //   - level 4 pass: reverse [24..27]   ("rac" -> "car")
        //   - level 3 pass: reverse [24..37]   (the bracketed RTL fragment)
        //   - level 2 pass: reverse [14..39]   (the LTR-inside-RTL embedding)
        //   - level 1 pass: reverse [0..42]    (the whole line)
        let visual = reorder_line(&lv);
        // After all four reversals, position 0 in visual order is
        // logical index 41 (the final paragraph-level-1 char), and
        // the algorithm should produce a strictly decreasing
        // prefix [41, 40, 39] followed by the inner-embedding
        // remap. Spot-check the head + tail:
        assert_eq!(visual[0], 41);
        assert_eq!(visual[1], 40);
        assert_eq!(visual[2], 39);
        assert_eq!(visual.last().copied(), Some(0));
        // And the permutation must be a valid permutation of 0..42.
        let mut sorted = visual.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..42).collect::<Vec<_>>());
    }

    #[test]
    fn l2_output_is_always_a_permutation() {
        // For any level vector the output must hit every index in
        // 0..n exactly once. Sweep a small set of mixed shapes.
        for lv in [
            vec![0u8, 1, 0, 1],
            vec![1, 0, 1, 0],
            vec![0, 2, 1, 2, 0],
            vec![3, 3, 1, 1, 3, 3],
            vec![5, 4, 3, 2, 1, 0],
            vec![0],
            vec![1],
            vec![0, 0, 0, 0, 0, 0, 0, 0],
        ] {
            let n = lv.len();
            let visual = reorder_line(&lv);
            assert_eq!(visual.len(), n);
            let mut sorted = visual.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, (0..n).collect::<Vec<_>>());
        }
    }

    #[test]
    fn l2_reverse_is_idempotent_when_applied_to_uniform_levels() {
        // Reordering an already-uniform-level line a second time
        // (by re-feeding the same level vector) would flip again —
        // the permutation is not its own inverse for n > 2. Sanity
        // check that the single application produces the expected
        // shape rather than a stable identity by accident.
        let lv = [1u8, 1, 1, 1, 1];
        let visual = reorder_line(&lv);
        assert_eq!(visual, vec![4, 3, 2, 1, 0]);
    }

    #[test]
    fn l1_then_l2_pipeline_trailing_space_in_rtl_paragraph() {
        // End-to-end mini-pipeline: L1 anchors trailing WS to the
        // paragraph level, then L2 reorders. With RTL text "AB " in
        // an RTL paragraph the displayed order should still have
        // the space on the visual left edge (paragraph-direction
        // tail) of the run.
        let cls = vec![BidiClass::R, BidiClass::R, BidiClass::WS];
        let mut lvl = vec![1, 1, 2];
        reset_trailing_levels(&cls, &mut lvl, 1);
        assert_eq!(lvl, vec![1, 1, 1]);
        let visual = reorder_line(&lvl);
        // All level 1 → full reverse. Visual order: WS, B, A.
        assert_eq!(visual, vec![2, 1, 0]);
    }
}
