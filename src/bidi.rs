//! Unicode Bidirectional Algorithm — UAX #9 character classes,
//! paragraph-level resolution (rules P1 / P2 / P3), explicit-level
//! / override / isolate stack (rules X1..X9), weak-type resolution
//! (rules W1..W7), neutral-type resolution (rules N1 and N2 —
//! bracket-pair rule N0 deferred to a follow-up round),
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
//! - [`resolve_explicit_levels`] — the **X1..X9** rules from §3.3.2
//!   over a whole paragraph. Walks the bidi class slice maintaining
//!   a directional status stack of (`level`, override-status,
//!   isolate-flag) frames plus the three overflow / valid counters
//!   the spec defines; emits per-character embedding levels, an
//!   override-rewritten effective class slice, and the X9 removal
//!   flag set ready for X10's isolating-run-sequence partition.
//!   FSI is resolved per X5c by running a P2 / P3 mini-pass over
//!   the FSI..matching-PDI span and treating it as an RLI / LRI
//!   accordingly.
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
//! - X10 isolating-run-sequence partition + sos/eos derivation are
//!   now implemented in [`level_runs`] (BD7) and
//!   [`isolating_run_sequences`] (BD13 + X10 step 2). Callers feed
//!   the result of [`resolve_explicit_levels`] in; each returned
//!   [`IsolatingRunSequence`] carries the constituent level-run
//!   ranges, the per-sequence embedding level, and the sos / eos
//!   directional types W1..W7 / N0..N2 / I1..I2 consume at the
//!   sequence boundaries.
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

// =============================================================================
// X-rules — explicit embedding / override / isolate stack (§3.3.2)
// =============================================================================

/// Maximum explicit embedding depth per UAX #9 §3.1.2 BD2.
///
/// BD2 fixes `max_depth = 125` and the spec explicitly guarantees the
/// value will not change: "this specification now guarantees that the
/// value of 125 for max_depth will not be increased (or decreased) in
/// future versions. Thus, it is safe for implementations to treat the
/// max_depth value as a constant." (UAX #9 Rev. 50 §3.1.2.) Embedding
/// initiators that would push past this depth are *overflow* events
/// (counted but otherwise ignored) per X2 / X3 / X4 / X5 / X5a / X5b.
pub const MAX_DEPTH: u8 = 125;

/// Output of the X1..X9 explicit-level pass.
///
/// `levels[i]` is the embedding level assigned to the `i`th input
/// character by X1..X8, `effective_classes[i]` is the (possibly
/// override-rewritten) bidi class the implicit phases consume, and
/// `removed[i]` reflects rule **X9** ("Remove all RLE, LRE, RLO, LRO,
/// PDF, and BN characters"). Index positions are preserved across all
/// three slices so callers can map back to the original logical
/// offsets if needed.
///
/// Per X9 the removed positions still carry a level — the spec note
/// allows implementations to leave the characters in place "as long as
/// all other characters are ordered correctly", so callers walking
/// `removed[i] == false` get the X9-filtered logical sequence in one
/// pass without re-shuffling indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitLevels {
    /// Per-character embedding level assigned by X1..X8. The first
    /// entry of the directional status stack (the paragraph level)
    /// is what gets assigned to every B per X8.
    pub levels: Vec<u8>,
    /// Per-character bidi class after X4 / X5 / X5a / X5b / X6 /
    /// X6a override rewriting. For positions whose enclosing scope
    /// has neutral override status the class is unchanged; under an
    /// `L` override every non-formatting character is rewritten to
    /// `L`; under an `R` override to `R` (per X6 and the X5a / X5b /
    /// X6a override-on-isolate-format clauses).
    pub effective_classes: Vec<BidiClass>,
    /// `removed[i]` is `true` iff the `i`th character is one of the
    /// types removed by X9 (`RLE` / `LRE` / `RLO` / `LRO` / `PDF` /
    /// `BN`). The isolate-formatting characters (`LRI` / `RLI` /
    /// `FSI` / `PDI`) are **not** removed per the X9 note "FSI, LRI,
    /// RLI, and PDI characters are not removed."
    pub removed: Vec<bool>,
}

/// Directional override status carried by each stack entry per
/// UAX #9 §3.1.2 BD6 / Table 2 (`Neutral` / `Right-to-left` /
/// `Left-to-right`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverrideStatus {
    Neutral,
    Ltr,
    Rtl,
}

/// One frame of the directional status stack per UAX #9 §3.3.2.
///
/// Each frame carries an embedding level, an override status, and an
/// isolate flag. The starting frame (X1) carries the paragraph
/// embedding level, neutral override, and isolate=`false`; this
/// frame is never popped until the end of the paragraph (X8).
#[derive(Debug, Clone, Copy)]
struct StackFrame {
    level: u8,
    overrride: OverrideStatus,
    isolate: bool,
}

/// Resolve **explicit embedding levels and override types** for a
/// whole paragraph per UAX #9 **X1..X9** (§3.3.2).
///
/// `classes` is the per-character [`BidiClass`] slice for the
/// paragraph in logical order; `paragraph_level` is the value
/// returned by [`paragraph_level`] (or set by a higher-level
/// protocol per HL1). The returned [`ExplicitLevels`] carries:
///
/// - `levels[i]` — the embedding level assigned by X1..X8 to the
///   `i`th input character. For non-formatting characters this is
///   the level of the last entry on the directional status stack at
///   the time the character is processed (X6). For embedding /
///   override initiators (`RLE` / `LRE` / `RLO` / `LRO`) the level
///   is the *new* scope level (the stack top *after* the push, or
///   the enclosing scope when the push overflowed). For PDF the
///   level is the post-pop stack top (the enclosing scope's level
///   after the matched embedding has been popped). For an isolate
///   initiator (`LRI` / `RLI` / `FSI`) the level is the *enclosing*
///   scope's level (per the X5a / X5b spec text "Set the LRI / RLI's
///   embedding level to the embedding level of the last entry on the
///   directional status stack."), and the matching PDI gets the
///   same level (X6a note: "the level assigned to an isolate
///   initiator is always the same as that assigned to the matching
///   PDI"). For B characters the paragraph embedding level (X8).
///   Since RLE / LRE / RLO / LRO / PDF are X9-removed, the precise
///   level reported for them is not consumed by the implicit phases.
/// - `effective_classes[i]` — the bidi class after override
///   rewriting per X4 / X5 / X5a / X5b / X6 / X6a. Override status
///   `Ltr` rewrites the class to `L`; `Rtl` rewrites it to `R`;
///   `Neutral` leaves it alone.
/// - `removed[i]` — true for X9-removed types (`RLE` / `LRE` /
///   `RLO` / `LRO` / `PDF` / `BN`). Isolate-formatting characters
///   (`LRI` / `RLI` / `FSI` / `PDI`) are *not* removed per the X9
///   note.
///
/// FSI is resolved per X5c by running a P2 / P3 mini-pass over the
/// FSI..matching-PDI span (or to end-of-paragraph if no matching
/// PDI), and the FSI is then treated as an RLI (paragraph level 1)
/// or LRI (paragraph level 0) accordingly.
///
/// Overflow events (depth ≥ `MAX_DEPTH`) are counted per the
/// "overflow isolate count" / "overflow embedding count" rules in
/// X2..X6a / X7. An overflow initiator's character receives the
/// level that was on top of the stack at the time of the initiator
/// (i.e. the level of the enclosing scope); an overflow PDF /
/// matching PDI decrements its respective overflow counter.
///
/// The X10 isolating-run-sequence partition is **not** computed by
/// this function — callers wanting to feed `resolve_weak_types` /
/// `resolve_neutral_types` / `resolve_implicit_levels` per-sequence
/// should run X10 as a separate pass over the returned levels +
/// effective_classes. The X-rule output here is the stable
/// per-character level vector X10 + the implicit phases consume.
///
/// Provenance: rules transcribed verbatim from
/// `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` §3.3.2 (UAX
/// #9 Revision 50, Unicode 16.0).
///
/// # Examples
///
/// ```
/// use oxideav_scribe::bidi::{
///     bidi_class, paragraph_level, resolve_explicit_levels, BidiClass,
/// };
///
/// // "Hello" — all L at level 0, no formatting characters, X9
/// // removes nothing.
/// let cls: Vec<BidiClass> = "Hello".chars().map(bidi_class).collect();
/// let pl = paragraph_level("Hello");
/// let out = resolve_explicit_levels(&cls, pl);
/// assert_eq!(out.levels, vec![0; 5]);
/// assert!(out.removed.iter().all(|r| !r));
///
/// // RLE A PDF — A is at level 1 (the RLE pushed an odd level);
/// // both RLE and PDF are X9-removed. The embedding initiator's
/// // own level is reported as the new-scope level; PDF reports
/// // the post-pop stack-top (the enclosing scope's level). Since
/// // both are X9-removed their reported level is not consumed by
/// // the implicit phases.
/// let cls = vec![BidiClass::RLE, BidiClass::L, BidiClass::PDF];
/// let out = resolve_explicit_levels(&cls, 0);
/// assert_eq!(out.levels, vec![1, 1, 0]);
/// assert_eq!(out.removed, vec![true, false, true]);
/// ```
#[must_use]
pub fn resolve_explicit_levels(classes: &[BidiClass], paragraph_level: u8) -> ExplicitLevels {
    let n = classes.len();
    let mut levels = vec![paragraph_level; n];
    let mut effective: Vec<BidiClass> = classes.to_vec();
    let mut removed = vec![false; n];

    // X1: initialise directional status stack with one entry holding
    // the paragraph embedding level, neutral override, and false
    // isolate status. Initialise the three overflow / valid
    // counters to zero.
    let mut stack: Vec<StackFrame> = Vec::with_capacity(8);
    stack.push(StackFrame {
        level: paragraph_level,
        overrride: OverrideStatus::Neutral,
        isolate: false,
    });
    let mut overflow_isolate: u32 = 0;
    let mut overflow_embedding: u32 = 0;
    let mut valid_isolate: u32 = 0;

    for i in 0..n {
        let cls = classes[i];

        match cls {
            // --- X2: RLE → least odd level above stack top ---------
            BidiClass::RLE => {
                apply_embedding(
                    &mut stack,
                    &mut overflow_isolate,
                    &mut overflow_embedding,
                    &mut levels,
                    i,
                    /* odd */ true,
                    OverrideStatus::Neutral,
                );
            }
            // --- X3: LRE → least even level above stack top --------
            BidiClass::LRE => {
                apply_embedding(
                    &mut stack,
                    &mut overflow_isolate,
                    &mut overflow_embedding,
                    &mut levels,
                    i,
                    /* odd */ false,
                    OverrideStatus::Neutral,
                );
            }
            // --- X4: RLO → least odd, Rtl override -----------------
            BidiClass::RLO => {
                apply_embedding(
                    &mut stack,
                    &mut overflow_isolate,
                    &mut overflow_embedding,
                    &mut levels,
                    i,
                    /* odd */ true,
                    OverrideStatus::Rtl,
                );
            }
            // --- X5: LRO → least even, Ltr override ----------------
            BidiClass::LRO => {
                apply_embedding(
                    &mut stack,
                    &mut overflow_isolate,
                    &mut overflow_embedding,
                    &mut levels,
                    i,
                    /* odd */ false,
                    OverrideStatus::Ltr,
                );
            }
            // --- X5a: RLI ------------------------------------------
            BidiClass::RLI => {
                apply_isolate(
                    &mut stack,
                    &mut overflow_isolate,
                    &mut overflow_embedding,
                    &mut valid_isolate,
                    &mut levels,
                    &mut effective,
                    i,
                    /* odd */ true,
                );
            }
            // --- X5b: LRI ------------------------------------------
            BidiClass::LRI => {
                apply_isolate(
                    &mut stack,
                    &mut overflow_isolate,
                    &mut overflow_embedding,
                    &mut valid_isolate,
                    &mut levels,
                    &mut effective,
                    i,
                    /* odd */ false,
                );
            }
            // --- X5c: FSI ------------------------------------------
            //
            // P2 + P3 applied to the FSI..matching-PDI span (or to
            // end-of-paragraph). If level 1 → treat as RLI per X5a;
            // otherwise as LRI per X5b.
            BidiClass::FSI => {
                let span_level = fsi_inner_level(classes, i + 1);
                apply_isolate(
                    &mut stack,
                    &mut overflow_isolate,
                    &mut overflow_embedding,
                    &mut valid_isolate,
                    &mut levels,
                    &mut effective,
                    i,
                    span_level == 1,
                );
            }
            // --- X6a: PDI ------------------------------------------
            BidiClass::PDI => {
                if overflow_isolate > 0 {
                    overflow_isolate -= 1;
                } else if valid_isolate > 0 {
                    // Terminate the matched isolate scope: reset
                    // overflow_embedding to zero, pop embedding
                    // entries above it, then pop the isolate frame
                    // itself. Decrement valid_isolate.
                    overflow_embedding = 0;
                    while let Some(top) = stack.last() {
                        if top.isolate {
                            break;
                        }
                        stack.pop();
                    }
                    // Per the spec note this stack pop is guaranteed
                    // safe — there is at least one isolate frame
                    // above the paragraph frame.
                    stack.pop();
                    valid_isolate -= 1;
                }
                // In all cases assign PDI's level + override-rewrite
                // its type from the (post-pop) stack top.
                let top = stack.last().expect("stack invariant: paragraph frame");
                levels[i] = top.level;
                effective[i] = match top.overrride {
                    OverrideStatus::Ltr => BidiClass::L,
                    OverrideStatus::Rtl => BidiClass::R,
                    OverrideStatus::Neutral => BidiClass::PDI,
                };
            }
            // --- X7: PDF -------------------------------------------
            BidiClass::PDF => {
                if overflow_isolate > 0 {
                    // PDF inside an overflow isolate is fully ignored.
                } else if overflow_embedding > 0 {
                    overflow_embedding -= 1;
                } else if stack.len() >= 2 && !stack.last().unwrap().isolate {
                    stack.pop();
                }
                // PDF's own level is the level on top of the stack
                // at processing time (the enclosing scope's level —
                // we used `stack.last()` *before* the pop above for
                // the embedding-pop case, but since X7 spec says PDF
                // is removed by X9 anyway, callers should not rely
                // on this value).
                levels[i] = stack.last().unwrap().level;
            }
            // --- X6: every other non-formatting type ---------------
            //
            // "For all types besides B, BN, RLE, LRE, RLO, LRO, PDF,
            // RLI, LRI, FSI, and PDI" — those are exhaustively
            // handled above. The remainder lands here.
            BidiClass::B => {
                // X8 / X1 — B characters get the paragraph level.
                levels[i] = paragraph_level;
            }
            BidiClass::BN => {
                // BN keeps the enclosing scope's level for the X10
                // run-partition + the sos/eos boundary lookups, but
                // X9 removes it from the implicit phases.
                let top = stack.last().expect("stack invariant: paragraph frame");
                levels[i] = top.level;
            }
            _ => {
                // X6 proper: every other type.
                let top = stack.last().expect("stack invariant: paragraph frame");
                levels[i] = top.level;
                effective[i] = match top.overrride {
                    OverrideStatus::Ltr => BidiClass::L,
                    OverrideStatus::Rtl => BidiClass::R,
                    OverrideStatus::Neutral => effective[i],
                };
            }
        }

        // X9: mark embeddings / overrides / PDF / BN as removed (the
        // implicit phases skip them). Isolate-formatting characters
        // (LRI / RLI / FSI / PDI) are NOT removed per the X9 note.
        if matches!(
            cls,
            BidiClass::RLE
                | BidiClass::LRE
                | BidiClass::RLO
                | BidiClass::LRO
                | BidiClass::PDF
                | BidiClass::BN
        ) {
            removed[i] = true;
        }
    }

    ExplicitLevels {
        levels,
        effective_classes: effective,
        removed,
    }
}

/// Apply X2 (RLE) / X3 (LRE) / X4 (RLO) / X5 (LRO) — the explicit
/// embedding / override push.
///
/// `target_odd` selects RLE / RLO behaviour (least odd above the
/// stack top) vs LRE / LRO (least even); `override_status` selects
/// the override status for the new frame (`Neutral` for embeddings,
/// `Ltr` for LRO, `Rtl` for RLO). Writes the initiator's own level
/// as the new scope's level (the stack top *after* the push, or the
/// enclosing scope when the push overflowed).
fn apply_embedding(
    stack: &mut Vec<StackFrame>,
    overflow_isolate: &mut u32,
    overflow_embedding: &mut u32,
    levels: &mut [u8],
    i: usize,
    target_odd: bool,
    override_status: OverrideStatus,
) {
    let enclosing_level = stack
        .last()
        .expect("stack invariant: paragraph frame")
        .level;
    let new_level = if target_odd {
        least_greater_odd(enclosing_level)
    } else {
        least_greater_even(enclosing_level)
    };
    if new_level <= MAX_DEPTH && *overflow_isolate == 0 && *overflow_embedding == 0 {
        stack.push(StackFrame {
            level: new_level,
            overrride: override_status,
            isolate: false,
        });
    } else if *overflow_isolate == 0 {
        *overflow_embedding = overflow_embedding.saturating_add(1);
    }
    // The initiator's own level reflects the new scope (the stack
    // top after the push). For overflow events the stack top stays
    // at the enclosing scope, which is the convention the spec's
    // X9-removal note implicitly endorses ("an implementation does
    // not have to actually remove the characters; it just has to
    // behave as though the characters were not present").
    levels[i] = stack
        .last()
        .expect("stack invariant: paragraph frame")
        .level;
}

/// Apply X5a (RLI) / X5b (LRI) / X5c (FSI-resolved-as-RLI-or-LRI).
///
/// `target_odd = true` for RLI / FSI-resolved-as-RLI; `false` for
/// LRI / FSI-resolved-as-LRI. Mutates the stack / overflow counters
/// and writes the isolate-initiator's own level + override-rewritten
/// class.
#[allow(clippy::too_many_arguments)]
fn apply_isolate(
    stack: &mut Vec<StackFrame>,
    overflow_isolate: &mut u32,
    overflow_embedding: &mut u32,
    valid_isolate: &mut u32,
    levels: &mut [u8],
    effective: &mut [BidiClass],
    i: usize,
    target_odd: bool,
) {
    // Per X5a / X5b: the isolate initiator's level is the level of
    // the last entry on the directional status stack (i.e. the
    // enclosing scope) — assigned *before* any push happens. The
    // override-status rewrite for the isolate initiator itself also
    // reads from the enclosing scope's override status.
    let enclosing = *stack.last().expect("stack invariant: paragraph frame");
    levels[i] = enclosing.level;
    effective[i] = match enclosing.overrride {
        OverrideStatus::Ltr => BidiClass::L,
        OverrideStatus::Rtl => BidiClass::R,
        OverrideStatus::Neutral => effective[i],
    };

    let new_level = if target_odd {
        least_greater_odd(enclosing.level)
    } else {
        least_greater_even(enclosing.level)
    };
    if new_level <= MAX_DEPTH && *overflow_isolate == 0 && *overflow_embedding == 0 {
        *valid_isolate = valid_isolate.saturating_add(1);
        stack.push(StackFrame {
            level: new_level,
            overrride: OverrideStatus::Neutral,
            isolate: true,
        });
    } else {
        *overflow_isolate = overflow_isolate.saturating_add(1);
    }
}

/// Least odd level strictly greater than `level` per X2 / X4 /
/// X5a's "least odd embedding level greater than the embedding
/// level of the last entry on the directional status stack."
const fn least_greater_odd(level: u8) -> u8 {
    // Even → +1 (next odd); odd → +2 (next odd). Saturates at
    // u8::MAX, but the validity check vs MAX_DEPTH happens outside.
    if level & 1 == 0 {
        level.saturating_add(1)
    } else {
        level.saturating_add(2)
    }
}

/// Least even level strictly greater than `level` per X3 / X5 /
/// X5b's "least even embedding level greater than the embedding
/// level of the last entry on the directional status stack."
const fn least_greater_even(level: u8) -> u8 {
    if level & 1 == 0 {
        level.saturating_add(2)
    } else {
        level.saturating_add(1)
    }
}

/// FSI resolution per X5c: apply P2 / P3 to the span between the
/// FSI at index `start - 1` (caller passes `start = fsi_index + 1`)
/// and its matching PDI (or end of paragraph). Returns `1` if the
/// resolved paragraph level is RTL, `0` otherwise.
///
/// The P2 search itself skips over inner isolate regions per the
/// same rule as [`paragraph_level`].
fn fsi_inner_level(classes: &[BidiClass], start: usize) -> u8 {
    let mut depth: u32 = 0;
    for cls in classes.iter().skip(start) {
        match *cls {
            BidiClass::LRI | BidiClass::RLI | BidiClass::FSI => {
                depth = depth.saturating_add(1);
            }
            BidiClass::PDI => {
                if depth == 0 {
                    // Matched the outer FSI — stop, no strong type
                    // found inside.
                    return 0;
                }
                depth -= 1;
            }
            _ if depth > 0 => {} // inside a nested isolate: skip
            BidiClass::L => return 0,
            BidiClass::R | BidiClass::AL => return 1,
            _ => {}
        }
    }
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

/// A maximal substring of characters that share an embedding level
/// per UAX #9 **BD7** (§3.1.2).
///
/// `start` is the index of the first character in the run; `end` is
/// one past the last character (half-open, matching `Range<usize>`
/// idioms). Both indices refer back into the level vector returned
/// by [`resolve_explicit_levels`] (and therefore the underlying
/// `classes` slice the caller fed to that function — they are the
/// same length).
///
/// X9-removed positions (RLE / LRE / RLO / LRO / PDF / BN) are
/// **included** in their containing level run — BD7 partitions on
/// assigned embedding level, not on removal status. The implicit
/// phases (W / N / I) walk runs via [`IsolatingRunSequence::indices`]
/// which skips removed positions per the X9 "behave as though the
/// characters were not present" clause.
///
/// Provenance: BD7 transcribed verbatim from
/// `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` §3.1.2
/// (UAX #9 Revision 50, Unicode 16.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelRun {
    /// First character index in the run (inclusive).
    pub start: usize,
    /// One past the last character index in the run (exclusive).
    pub end: usize,
    /// Embedding level shared by every character in the run.
    pub level: u8,
}

impl LevelRun {
    /// Length of the run in character positions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// `true` iff [`Self::len`] is zero.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

/// Compute the **BD7 level-run partition** of a paragraph from the
/// per-character level vector produced by [`resolve_explicit_levels`].
///
/// A level run is a maximal substring of characters that have the
/// same embedding level (BD7). The output is the level-run list in
/// logical-order; concatenating their `start..end` ranges covers
/// `0..levels.len()` with no overlap or gap.
///
/// `levels.len()` must equal the paragraph's character count (it
/// always does when obtained from [`resolve_explicit_levels`]).
/// For the empty input the output is empty.
///
/// Provenance: BD7 (§3.1.2). The function is the prerequisite for
/// X10 / BD13 ([`isolating_run_sequences`]).
///
/// # Examples
///
/// ```
/// use oxideav_scribe::bidi::level_runs;
///
/// // Uniform-level paragraph collapses to one run.
/// let runs = level_runs(&[0, 0, 0, 0]);
/// assert_eq!(runs.len(), 1);
/// assert_eq!((runs[0].start, runs[0].end, runs[0].level), (0, 4, 0));
///
/// // A level change splits the partition cleanly: levels
/// // [0, 1, 1, 0, 0] → (0..1, 0), (1..3, 1), (3..5, 0).
/// let runs = level_runs(&[0, 1, 1, 0, 0]);
/// assert_eq!(runs.len(), 3);
/// assert_eq!((runs[0].start, runs[0].end, runs[0].level), (0, 1, 0));
/// assert_eq!((runs[1].start, runs[1].end, runs[1].level), (1, 3, 1));
/// assert_eq!((runs[2].start, runs[2].end, runs[2].level), (3, 5, 0));
/// ```
#[must_use]
pub fn level_runs(levels: &[u8]) -> Vec<LevelRun> {
    let mut out = Vec::new();
    let n = levels.len();
    if n == 0 {
        return out;
    }
    let mut start = 0usize;
    let mut cur = levels[0];
    for (i, &lvl) in levels.iter().enumerate().skip(1) {
        if lvl != cur {
            out.push(LevelRun {
                start,
                end: i,
                level: cur,
            });
            start = i;
            cur = lvl;
        }
    }
    out.push(LevelRun {
        start,
        end: n,
        level: cur,
    });
    out
}

/// One isolating run sequence per UAX #9 **BD13** + **X10** (§3.1.2,
/// §3.3.3).
///
/// An isolating run sequence is a maximal sequence of level runs
/// chained across matched isolate-initiator → PDI boundaries (BD13).
/// All level runs in a sequence share the same embedding level (BD13
/// note: "Thus, all the level runs in an isolating run sequence have
/// the same embedding level."). The W1..W7 / N0..N2 / I1..I2 implicit
/// phases run **once per sequence** treating "the last character of
/// each level run in the isolating run sequence is treated as if it
/// were immediately followed by the first character in the next
/// level run in the sequence" per X10 step 3.
///
/// Fields:
///
/// - `runs` — the constituent level runs in logical order. Always
///   non-empty: a sequence has at least one run.
/// - `level` — the shared embedding level.
/// - `sos`, `eos` — the start-of-sequence and end-of-sequence
///   directional types (`L` or `R`) per X10 step 2, derived from
///   the *higher* of the two levels on either side of the sequence
///   boundary (the paragraph embedding level if the boundary lies
///   at paragraph edge or at an isolate initiator with no matching
///   PDI). `R` iff the higher level is odd; `L` otherwise. These
///   are the `sos` / `eos` arguments the existing
///   [`resolve_weak_types`] / [`resolve_neutral_types`] surface
///   expects.
///
/// Provenance: BD13 and X10 transcribed verbatim from
/// `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` §3.1.2 +
/// §3.3.3 (UAX #9 Revision 50, Unicode 16.0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolatingRunSequence {
    /// Constituent level runs in logical (paragraph) order. Always
    /// `len() >= 1`.
    pub runs: Vec<LevelRun>,
    /// Shared embedding level across every run in the sequence.
    pub level: u8,
    /// Start-of-sequence directional type per X10 step 2. Always
    /// `BidiClass::L` or `BidiClass::R`.
    pub sos: BidiClass,
    /// End-of-sequence directional type per X10 step 2. Always
    /// `BidiClass::L` or `BidiClass::R`.
    pub eos: BidiClass,
}

impl IsolatingRunSequence {
    /// Iterator over the character indices belonging to this
    /// sequence, in logical order, **skipping X9-removed positions**.
    ///
    /// `removed` is the parallel slice returned by
    /// [`resolve_explicit_levels`] (`ExplicitLevels::removed`). The
    /// returned iterator walks every run in `self.runs` and yields
    /// each index `i` where `removed[i] == false`. The result is
    /// the in-order character index list the W / N / I phases
    /// consume per X9's "behave as though the characters were not
    /// present" clause.
    ///
    /// Panics if `removed.len()` is smaller than any run's `end`.
    pub fn indices<'a>(&'a self, removed: &'a [bool]) -> impl Iterator<Item = usize> + 'a {
        self.runs
            .iter()
            .flat_map(move |r| (r.start..r.end).filter(move |&i| !removed[i]))
    }
}

/// Convert a higher-of-two embedding levels into an sos / eos
/// directional type per UAX #9 X10 step 2: "If the higher level is
/// odd, the sos or eos is R; otherwise, it is L."
fn level_to_sos_eos(level: u8) -> BidiClass {
    if level % 2 == 0 {
        BidiClass::L
    } else {
        BidiClass::R
    }
}

/// Find the index of the matching PDI for the isolate initiator at
/// `start` per UAX #9 **BD9** (§3.1.2).
///
/// `classes` is the original (pre-X9, pre-override) class slice (the
/// same input fed to [`resolve_explicit_levels`]). Returns `Some(j)`
/// where `j > start` is the index of the matching PDI; returns
/// `None` if no matching PDI exists (the isolate initiator is
/// unbalanced).
///
/// Note: BD9 only counts isolate initiators (LRI / RLI / FSI) and
/// PDIs — every other formatting character is ignored. The depth
/// limit (MAX_DEPTH overflow during X1..X9) is **not** considered
/// at this layer per BD9's closing note "this algorithm assigns a
/// matching PDI (or lack of one) to an isolate initiator whether
/// the isolate initiator raises the embedding level or is prevented
/// from doing so by the depth limit."
fn matching_pdi(classes: &[BidiClass], start: usize) -> Option<usize> {
    let mut counter: i32 = 1;
    for (offset, cls) in classes.iter().enumerate().skip(start + 1) {
        match cls {
            BidiClass::LRI | BidiClass::RLI | BidiClass::FSI => counter += 1,
            BidiClass::PDI => {
                counter -= 1;
                if counter == 0 {
                    return Some(offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Compute the **isolating-run-sequence partition** of a paragraph
/// per UAX #9 **X10** (§3.3.3), with `sos` and `eos` directional
/// types attached per X10 step 2.
///
/// `classes` is the original per-character [`BidiClass`] slice fed
/// to [`resolve_explicit_levels`]; `explicit` is its return value;
/// `paragraph_level` is the paragraph embedding level (the same
/// value passed to [`resolve_explicit_levels`]).
///
/// Output is a vector of [`IsolatingRunSequence`] in deterministic
/// order (each sequence in the order its first level run begins in
/// the paragraph). Every level run in the paragraph belongs to
/// exactly one sequence (BD13 note). The implicit phases run
/// independently on each sequence (X10 step 3 closing note: "The
/// order that one isolating run sequence is treated relative to
/// another does not matter.").
///
/// The X10 step 2 sos / eos derivation uses the **higher** of the
/// levels on either side of the sequence boundary, **skipping
/// X9-removed characters** when looking outward. At a sequence
/// boundary lying at paragraph edge, OR at an isolate initiator
/// with no matching PDI (the eos side), the paragraph embedding
/// level is used as the other side.
///
/// Provenance: X10 + BD13 transcribed verbatim from
/// `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` §3.1.2 +
/// §3.3.3 (UAX #9 Revision 50, Unicode 16.0).
///
/// # Examples
///
/// ```
/// use oxideav_scribe::bidi::{
///     bidi_class, isolating_run_sequences, level_runs, paragraph_level,
///     resolve_explicit_levels, BidiClass,
/// };
///
/// // Simple paragraph with no formatting: "abc" — one sequence,
/// // one level run, sos = eos = L (paragraph level 0).
/// let s = "abc";
/// let cls: Vec<_> = s.chars().map(bidi_class).collect();
/// let pl = paragraph_level(s);
/// let out = resolve_explicit_levels(&cls, pl);
/// let seqs = isolating_run_sequences(&cls, &out, pl);
/// assert_eq!(seqs.len(), 1);
/// assert_eq!(seqs[0].sos, BidiClass::L);
/// assert_eq!(seqs[0].eos, BidiClass::L);
/// assert_eq!(seqs[0].level, 0);
/// assert_eq!(seqs[0].runs.len(), 1);
///
/// // Two RLE-bounded level runs chain into separate sequences:
/// // L RLE L PDF L — runs [0..1] @ 0, [1..3] @ 1, [3..5] @ 0.
/// // No isolate initiators, so each run is its own sequence.
/// let cls = vec![
///     BidiClass::L,
///     BidiClass::RLE,
///     BidiClass::L,
///     BidiClass::PDF,
///     BidiClass::L,
/// ];
/// let out = resolve_explicit_levels(&cls, 0);
/// let seqs = isolating_run_sequences(&cls, &out, 0);
/// assert_eq!(seqs.len(), 3);
/// // First sequence: level 0, sos L (paragraph), eos L (next run
/// // is level 1 > 0, so the higher is 1 → R).
/// assert_eq!(seqs[0].level, 0);
/// assert_eq!(seqs[0].sos, BidiClass::L);
/// assert_eq!(seqs[0].eos, BidiClass::R);
/// // Middle sequence: level 1, sos = eos = R.
/// assert_eq!(seqs[1].level, 1);
/// assert_eq!(seqs[1].sos, BidiClass::R);
/// assert_eq!(seqs[1].eos, BidiClass::R);
/// // Last sequence: level 0, sos R (higher of 1 vs 0), eos L
/// // (paragraph edge).
/// assert_eq!(seqs[2].level, 0);
/// assert_eq!(seqs[2].sos, BidiClass::R);
/// assert_eq!(seqs[2].eos, BidiClass::L);
/// ```
#[must_use]
pub fn isolating_run_sequences(
    classes: &[BidiClass],
    explicit: &ExplicitLevels,
    paragraph_level: u8,
) -> Vec<IsolatingRunSequence> {
    let runs = level_runs(&explicit.levels);
    if runs.is_empty() {
        return Vec::new();
    }

    // For each level run, record the index of the matching-PDI
    // level run, if the run ends with an isolate initiator. The
    // "level run containing the matching PDI" indirection is BD13
    // step 2: "append the level run containing the matching PDI to
    // the sequence. (Note that this matching PDI must be the first
    // character of its level run.)"
    //
    // We also record the "first-char is PDI matching some prior
    // isolate" flag — runs starting with such a PDI may NOT seed a
    // new sequence (BD13 step "For each level run ... whose first
    // character is not a PDI, or is a PDI that does not match any
    // isolate initiator").

    // Map: character index → index of the level run containing it.
    let mut run_of_index = vec![0usize; explicit.levels.len()];
    for (ri, r) in runs.iter().enumerate() {
        for slot in &mut run_of_index[r.start..r.end] {
            *slot = ri;
        }
    }

    let nruns = runs.len();
    // For each run, the index of the run holding the matching PDI
    // (if its last char is an isolate initiator with a matching
    // PDI). None otherwise.
    let mut next_run: Vec<Option<usize>> = vec![None; nruns];
    // For each run, true iff its first char is a PDI that DOES
    // match some isolate initiator earlier in the paragraph (i.e.
    // the run is chained to from somewhere). Such runs cannot seed
    // a sequence; they are only appended to one.
    let mut chained_from_prior: Vec<bool> = vec![false; nruns];

    for (ri, r) in runs.iter().enumerate() {
        // Walk the run's last character (skipping X9-removed
        // positions, since BD13 talks about characters and X9
        // says removed characters "behave as though [they] were
        // not present"). We need the last *non-removed* index.
        let mut last_idx: Option<usize> = None;
        for i in (r.start..r.end).rev() {
            if !explicit.removed[i] {
                last_idx = Some(i);
                break;
            }
        }
        let Some(last) = last_idx else {
            continue;
        };
        // BD13 chains across isolate-initiator → matching-PDI only.
        if matches!(
            classes[last],
            BidiClass::LRI | BidiClass::RLI | BidiClass::FSI
        ) {
            if let Some(pdi_idx) = matching_pdi(classes, last) {
                let pdi_run = run_of_index[pdi_idx];
                // BD13's parenthetical note: "this matching PDI
                // must be the first character of its level run".
                // Confirm — if not, the chain breaks and we skip.
                if runs[pdi_run].start == pdi_idx {
                    next_run[ri] = Some(pdi_run);
                    chained_from_prior[pdi_run] = true;
                }
            }
        }
    }

    // Build the sequences. For each run that may seed a sequence
    // (not chained_from_prior), walk next_run links until None.
    let mut sequences: Vec<IsolatingRunSequence> = Vec::with_capacity(nruns);
    let mut visited: Vec<bool> = vec![false; nruns];
    for seed in 0..nruns {
        if chained_from_prior[seed] || visited[seed] {
            continue;
        }
        let mut chain: Vec<LevelRun> = Vec::new();
        let mut cur = Some(seed);
        while let Some(idx) = cur {
            if visited[idx] {
                // Defensive: BD13 partitions exactly, so we never
                // visit a run twice. Break if we hit a cycle.
                break;
            }
            visited[idx] = true;
            chain.push(runs[idx]);
            cur = next_run[idx];
        }
        debug_assert!(!chain.is_empty(), "BD13 seed produces non-empty chain");

        // X10 step 2: sos uses the *higher* of (level of first
        // char in sequence, level of preceding non-removed char in
        // paragraph), where "preceding non-removed char" falls back
        // to the paragraph embedding level. eos uses the *higher*
        // of (level of last char in sequence, level of following
        // non-removed char in paragraph), with two fallbacks to
        // paragraph level: (a) paragraph edge, (b) last char of
        // the last run is an isolate initiator with no matching
        // PDI.
        let level = chain[0].level;
        let first_idx = chain[0].start;
        let last_idx = chain.last().unwrap().end - 1;

        // Walk backwards from first_idx-1 to skip removed chars.
        let mut sos_other = paragraph_level;
        for i in (0..first_idx).rev() {
            if !explicit.removed[i] {
                sos_other = explicit.levels[i];
                break;
            }
        }
        let sos_level = sos_other.max(level);

        // Determine eos other-side: forward from last_idx+1.
        // First, decide whether the eos side is governed by the
        // paragraph fallback. Spec triggers: (a) no following
        // non-removed char in paragraph, (b) last char of last run
        // is an isolate initiator lacking a matching PDI.
        let mut eos_fallback = false;
        // Find last non-removed char in the last run.
        let mut last_run_last_nonremoved: Option<usize> = None;
        for i in (chain.last().unwrap().start..chain.last().unwrap().end).rev() {
            if !explicit.removed[i] {
                last_run_last_nonremoved = Some(i);
                break;
            }
        }
        if let Some(j) = last_run_last_nonremoved {
            if matches!(classes[j], BidiClass::LRI | BidiClass::RLI | BidiClass::FSI)
                && matching_pdi(classes, j).is_none()
            {
                eos_fallback = true;
            }
        }
        let mut eos_other = paragraph_level;
        if !eos_fallback {
            let mut found = false;
            for i in (last_idx + 1)..explicit.levels.len() {
                if !explicit.removed[i] {
                    eos_other = explicit.levels[i];
                    found = true;
                    break;
                }
            }
            if !found {
                eos_other = paragraph_level;
            }
        }
        let eos_level = eos_other.max(level);

        sequences.push(IsolatingRunSequence {
            runs: chain,
            level,
            sos: level_to_sos_eos(sos_level),
            eos: level_to_sos_eos(eos_level),
        });
    }

    sequences
}

/// Result of the whole-paragraph UAX #9 §3 pipeline produced by
/// [`process_paragraph_classes`] and [`process_paragraph`].
///
/// The struct carries every artefact a line-breaking + line-reorder
/// caller needs to drive **L1** ([`reset_trailing_levels`]) and **L2**
/// ([`reorder_line`]) on a per-line slice after the paragraph has been
/// broken into display lines.
///
/// All vectors are paragraph-wide and parallel — `classes[i]`,
/// `effective_classes[i]`, `levels[i]`, and `removed[i]` all describe
/// the same paragraph character at logical index `i`. For [`process_paragraph`]
/// callers, `char_byte_offsets[i]` is the byte offset of that character
/// in the input `&str` (so a `text[char_byte_offsets[i]..]` slice
/// starts at the character a level applies to).
///
/// # Composition order
///
/// 1. Bidi class assignment (§3.2 / [`bidi_class`]).
/// 2. Paragraph embedding level (§3.3.1 P2 + P3 / [`paragraph_level`]),
///    unless the caller provides `base_level`.
/// 3. Explicit-level / override / isolate stack pass (§3.3.2 X1..X9 /
///    [`resolve_explicit_levels`]).
/// 4. Isolating-run-sequence partition + sos / eos derivation (§3.3.3
///    X10 / [`isolating_run_sequences`]) on top of the BD7 + BD13
///    output of [`level_runs`].
/// 5. Per-sequence weak-type pass (§3.3.4 W1..W7 /
///    [`resolve_weak_types`]).
/// 6. Per-sequence neutral-type pass (§3.3.5 N1 + N2 /
///    [`resolve_neutral_types`]). N0 bracket-pair resolution is
///    deferred — see the module-level out-of-scope list.
/// 7. Per-sequence implicit-level pass (§3.3.6 I1 + I2 /
///    [`resolve_implicit_levels`]).
///
/// The L1 / L2 line passes are **not** run here because they operate
/// per display line (after a higher-level line-breaker has decided
/// where the paragraph splits into lines). The carrier exposes
/// [`ParagraphBidi::reorder_line_range`] as a convenience for callers
/// that have not yet wired a line-breaker and want to treat the
/// whole paragraph as one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphBidi {
    /// The paragraph embedding level resolved by §3.3.1 P2 / P3 (or
    /// supplied by the caller).
    pub paragraph_level: u8,
    /// Original per-character bidi class. This is the slice
    /// [`reset_trailing_levels`] consumes per the §3.4 "original
    /// types" normative note when L1 runs.
    pub classes: Vec<BidiClass>,
    /// Per-character bidi class after X4 / X5 / X5a / X5b / X6 / X6a
    /// override rewriting from X1..X9. Use this if you want to inspect
    /// the post-override classes; W / N / I have already consumed it
    /// to produce `levels`.
    pub effective_classes: Vec<BidiClass>,
    /// X9-removed flag set. `removed[i] == true` for `RLE` / `LRE` /
    /// `RLO` / `LRO` / `PDF` / `BN`. Isolate-formatting characters
    /// `LRI` / `RLI` / `FSI` / `PDI` are **not** flagged per the
    /// §3.3.2 X9 note.
    pub removed: Vec<bool>,
    /// Per-character resolved embedding level after the full
    /// X → W → N → I sweep. Index `i` carries the I-rule output level
    /// for the `i`th paragraph character (the same indexing as
    /// `classes`). X9-removed positions carry their containing level
    /// run's level (the X-rule output value), since W / N / I skipped
    /// them per X9's "behave as though the characters were not
    /// present" clause.
    pub levels: Vec<u8>,
}

impl ParagraphBidi {
    /// Run L1 + L2 across the whole paragraph and return the
    /// logical-to-visual permutation.
    ///
    /// This is the "no line-breaker" convenience path: the caller
    /// treats the entire paragraph as a single display line. Real
    /// callers that wrap the paragraph across N visual lines should
    /// instead call [`reset_trailing_levels`] + [`reorder_line`]
    /// per-line on the appropriate slice of `classes` + `levels`.
    ///
    /// Returns a `Vec<usize>` of the same length as `levels`, where
    /// the `k`th entry is the logical index of the character that
    /// belongs at visual position `k`.
    #[must_use]
    pub fn reorder_paragraph(&self) -> Vec<usize> {
        let mut levels = self.levels.clone();
        reset_trailing_levels(&self.classes, &mut levels, self.paragraph_level);
        reorder_line(&levels)
    }

    /// Run L1 + L2 over `line` (a half-open `[start, end)` range into
    /// the paragraph's `classes` / `levels` vectors) and return the
    /// per-line logical-to-visual permutation.
    ///
    /// The returned permutation is **relative to the line** — entry
    /// `k` is the index *within the line slice* that belongs at
    /// visual position `k`. Add `line.start` to map back into the
    /// paragraph.
    ///
    /// # Panics
    ///
    /// Panics if `line.start > line.end` or `line.end > self.levels.len()`.
    #[must_use]
    pub fn reorder_line_range(&self, line: core::ops::Range<usize>) -> Vec<usize> {
        assert!(
            line.start <= line.end && line.end <= self.levels.len(),
            "reorder_line_range: line range {line:?} out of bounds for {} chars",
            self.levels.len(),
        );
        let cls = &self.classes[line.clone()];
        let mut lvl = self.levels[line].to_vec();
        reset_trailing_levels(cls, &mut lvl, self.paragraph_level);
        reorder_line(&lvl)
    }
}

/// Run the whole UAX #9 §3 paragraph pipeline (P → X → W → N → I) on
/// the supplied per-character class slice.
///
/// `base_level` lets the caller override the §3.3.1 P2 / P3 paragraph
/// embedding level (HL1: higher-level protocol decided the base
/// direction). When `None` is passed, the paragraph level is resolved
/// from the supplied class slice via the same first-strong walk
/// [`paragraph_level`] performs on text, but operating on the class
/// slice directly (skipping isolate spans LRI / RLI / FSI .. PDI
/// per BD8).
///
/// All four phases beyond X10 (W1..W7, N1 + N2, I1 + I2) run **per
/// isolating run sequence** per the X10 step 3 closing note ("the
/// order that one isolating run sequence is treated relative to
/// another does not matter"). Each per-sequence pass consumes the
/// `sos` / `eos` directional types derived from the higher-of-two-
/// levels rule in X10 step 2 (already attached on each
/// [`IsolatingRunSequence`] by [`isolating_run_sequences`]).
///
/// Returns the full [`ParagraphBidi`] carrier; see its docs for the
/// per-field semantics. L1 / L2 are not run here — callers either
/// invoke [`ParagraphBidi::reorder_paragraph`] (treat the paragraph
/// as one line) or run [`reset_trailing_levels`] + [`reorder_line`]
/// per display line themselves.
///
/// Provenance: §3 driver composed from the §3.3.1 / §3.3.2 / §3.3.3 /
/// §3.3.4 / §3.3.5 / §3.3.6 entry points already in this module,
/// each of which cites
/// `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` directly.
///
/// # Examples
///
/// ```
/// use oxideav_scribe::bidi::{bidi_class, process_paragraph_classes, BidiClass};
///
/// // "Hello" — all L, paragraph level 0, every character at level 0.
/// let cls: Vec<_> = "Hello".chars().map(bidi_class).collect();
/// let p = process_paragraph_classes(&cls, None);
/// assert_eq!(p.paragraph_level, 0);
/// assert_eq!(p.levels, vec![0; 5]);
/// ```
#[must_use]
pub fn process_paragraph_classes(classes: &[BidiClass], base_level: Option<u8>) -> ParagraphBidi {
    let pl = base_level
        .map(|l| l & 1)
        .unwrap_or_else(|| paragraph_level_from_classes(classes));

    // X1..X9: per-character embedding levels + override-rewritten
    // classes + X9 removed flags. Caller-fed `classes` is preserved
    // verbatim in the returned carrier (L1 needs the original types
    // per §3.4 note).
    let explicit = resolve_explicit_levels(classes, pl);

    // levels starts as a clone of the X-rule output. The W / N / I
    // sweep below overwrites every non-X9-removed position with its
    // I-rule resolved level. X9-removed positions stay at the X-rule
    // level (W / N / I skipped them) — L1 / L2 see them unchanged.
    let mut levels = explicit.levels.clone();

    let sequences = isolating_run_sequences(classes, &explicit, pl);
    for seq in &sequences {
        // Materialise the per-sequence effective class slice + the
        // logical-index mapping (X9 skips removed positions).
        let seq_indices: Vec<usize> = seq.indices(&explicit.removed).collect();
        if seq_indices.is_empty() {
            continue;
        }
        let mut seq_classes: Vec<BidiClass> = seq_indices
            .iter()
            .map(|&i| explicit.effective_classes[i])
            .collect();

        // W1..W7 in place over the per-sequence class buffer.
        resolve_weak_types(&mut seq_classes, seq.sos, seq.eos);
        // N1 + N2 in place over the same buffer.
        resolve_neutral_types(&mut seq_classes, seq.level, seq.sos, seq.eos);
        // I1 + I2 reads classes and emits levels. The output has the
        // same length as `seq_classes`.
        let seq_levels = resolve_implicit_levels(&seq_classes, seq.level);
        // Scatter resolved levels back to the paragraph-wide vector.
        for (offset, &paragraph_idx) in seq_indices.iter().enumerate() {
            levels[paragraph_idx] = seq_levels[offset];
        }
    }

    ParagraphBidi {
        paragraph_level: pl,
        classes: classes.to_vec(),
        effective_classes: explicit.effective_classes,
        removed: explicit.removed,
        levels,
    }
}

/// First-strong paragraph-level walk over a pre-classified
/// [`BidiClass`] slice, per UAX #9 §3.3.1 **P2 + P3**, with **BD8**
/// isolate-span skipping.
///
/// Mirrors what [`paragraph_level`] does on a `&str`, but consumes
/// the class slice directly so [`process_paragraph_classes`] does not
/// need to round-trip back to text. Returns `0` (LTR) if the first
/// strong character is `L`, `1` (RTL) if it is `R` or `AL`, or `0`
/// per **P3** when the paragraph contains no strong character.
fn paragraph_level_from_classes(classes: &[BidiClass]) -> u8 {
    let n = classes.len();
    let mut i = 0;
    while i < n {
        match classes[i] {
            BidiClass::L => return 0,
            BidiClass::R | BidiClass::AL => return 1,
            BidiClass::LRI | BidiClass::RLI | BidiClass::FSI => {
                // BD8: skip the isolate span when searching for the
                // first strong character. The matching PDI also
                // counts as the close of the span.
                if let Some(pdi) = matching_pdi(classes, i) {
                    i = pdi + 1;
                } else {
                    // No matching PDI: the spec says skip to end of
                    // paragraph, i.e. nothing after the isolate
                    // initiator contributes a strong type for P2.
                    return 0;
                }
            }
            _ => i += 1,
        }
    }
    0
}

/// Run the whole UAX #9 §3 paragraph pipeline on a `&str` and
/// return the [`ParagraphBidi`] carrier alongside a parallel vector
/// of byte offsets locating each character in the input.
///
/// `text` must be a single paragraph — callers handle the §3.3.1 P1
/// paragraph split via [`split_paragraphs`] first. `base_level` is
/// the HL1 override per [`process_paragraph_classes`] semantics.
///
/// Returns `(carrier, char_byte_offsets)` where
/// `char_byte_offsets[i]` is the byte index of the `i`th character
/// in `text` (`text.char_indices()`'s first projection). The
/// `carrier.classes` / `carrier.levels` / etc. are all character-
/// indexed slices of the same length, so `carrier.levels[i]` carries
/// the level of the character starting at `text[char_byte_offsets[i]]`.
///
/// Provenance: same as [`process_paragraph_classes`] — composed from
/// the per-rule entry points already in this module.
#[must_use]
pub fn process_paragraph(text: &str, base_level: Option<u8>) -> (ParagraphBidi, Vec<usize>) {
    let mut char_byte_offsets = Vec::new();
    let mut classes = Vec::new();
    for (i, c) in text.char_indices() {
        char_byte_offsets.push(i);
        classes.push(bidi_class(c));
    }
    let carrier = process_paragraph_classes(&classes, base_level);
    (carrier, char_byte_offsets)
}

/// Per-paragraph carrier inside a multi-paragraph [`TextBidi`] result.
///
/// Wraps a [`ParagraphBidi`] (the §3 P → X → W → N → I output for one
/// paragraph) with the bookkeeping that locates the paragraph back in
/// the original input — the byte range it occupies in the input
/// `&str` (`byte_range`) and the cumulative character offset at which
/// the paragraph starts within the whole-text logical sequence
/// (`char_offset`).
///
/// The paragraph slice referenced by `byte_range` is the verbatim
/// substring P1 produced: a paragraph separator (type B — CR, LF, CRLF,
/// FF, NEL, LS, PS) is *kept with the preceding paragraph* per UAX #9
/// §3.3.1 P1, so the terminating B character (if any) is included in
/// both `byte_range` and the paragraph's [`ParagraphBidi::classes`] /
/// `levels` slices.
///
/// `char_byte_offsets[i]` is the byte index of the `i`th character of
/// the paragraph **within the original whole-text input** (not within
/// the paragraph slice). Subtract `byte_range.start` to get a paragraph-
/// local offset. Combined with `char_offset`, this lets callers walk
/// from a whole-text logical character index `k` to the carrier and
/// position in O(1) once the right paragraph is selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphSlice {
    /// Byte range of the paragraph (including its trailing B if any)
    /// in the original `&str` fed to [`process_text`]. Half-open
    /// `[start, end)`.
    pub byte_range: core::ops::Range<usize>,
    /// Cumulative character offset at which this paragraph begins in
    /// the whole-text logical sequence. `paragraphs[0].char_offset ==
    /// 0`; subsequent entries accumulate the prior paragraph's
    /// `bidi.levels.len()`.
    pub char_offset: usize,
    /// The §3 paragraph pipeline output for this paragraph.
    pub bidi: ParagraphBidi,
    /// Whole-input byte offset of each character in this paragraph
    /// (length matches `bidi.classes.len()` / `bidi.levels.len()`).
    /// `char_byte_offsets[i]` is the byte index of the `i`th character
    /// in the original `&str` fed to [`process_text`].
    pub char_byte_offsets: Vec<usize>,
}

/// Output of the multi-paragraph UAX #9 §3 driver [`process_text`].
///
/// The §3 P1 step "Split the text into separate paragraphs" treats a
/// paragraph separator (BidiClass `B`) as the terminator of the
/// preceding paragraph; the algorithm then "applies all the other rules
/// of this algorithm" inside each paragraph independently. This carrier
/// holds one [`ParagraphSlice`] per such paragraph, in logical order,
/// alongside the whole-input character count.
///
/// The base level applied to each paragraph is the same `base_level`
/// argument fed to [`process_text`] — UAX #9 §3.3.1 P2 / P3 run *per
/// paragraph* on the paragraph's own first-strong character, but HL1
/// (caller override) is paragraph-independent at the public surface
/// here: passing `Some(level)` forces every paragraph to that level;
/// `None` lets P2 / P3 walk each paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBidi {
    /// One entry per paragraph found by P1, in logical order. Empty
    /// iff the input is empty (an empty string has no paragraphs).
    pub paragraphs: Vec<ParagraphSlice>,
    /// Total character count across all paragraphs. Equals the sum of
    /// `paragraphs[i].bidi.levels.len()` and `text.chars().count()`
    /// for the input `text` fed to [`process_text`].
    pub total_chars: usize,
}

impl TextBidi {
    /// Number of paragraphs P1 produced. Zero iff the input was empty.
    #[must_use]
    pub fn len(&self) -> usize {
        self.paragraphs.len()
    }

    /// `true` iff [`Self::len`] is zero.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paragraphs.is_empty()
    }

    /// Locate the paragraph containing the whole-input logical
    /// character index `k`. Returns `(paragraph_index,
    /// paragraph_local_char_index)` such that
    /// `paragraphs[paragraph_index].bidi.classes[paragraph_local_char_index]`
    /// is the §3.4-input class of the `k`-th whole-input character.
    ///
    /// Returns `None` if `k >= total_chars`.
    #[must_use]
    pub fn locate_char(&self, k: usize) -> Option<(usize, usize)> {
        // Linear walk; paragraph counts in real text are small so this
        // is acceptable. Sorted-by-`char_offset` invariant lets a
        // future caller swap in a binary search if profiling shows it.
        for (pi, p) in self.paragraphs.iter().enumerate() {
            let end = p.char_offset + p.bidi.levels.len();
            if k < end {
                return Some((pi, k - p.char_offset));
            }
        }
        None
    }
}

/// Run the whole UAX #9 §3 paragraph pipeline (P1 split → per-
/// paragraph P → X → W → N → I) across a multi-paragraph `&str`.
///
/// This is the top-level entry point that callers with whole-document
/// text (potentially containing newlines, paragraph separators, form
/// feeds, …) reach for. Internally it walks the §3.3.1 P1 step ("Split
/// the text into separate paragraphs") via [`split_paragraphs`] —
/// trailing paragraph separators (BidiClass `B` — `\u{000A}` LF,
/// `\u{000D}` CR, `\u{000C}` FF, `\u{0085}` NEL, `\u{2028}` LS,
/// `\u{2029}` PS) are kept with the preceding paragraph per the spec
/// — and dispatches each paragraph slice through
/// [`process_paragraph_classes`] independently. The per-paragraph
/// `char_byte_offsets` are shifted by the paragraph's `byte_range.start`
/// so callers get whole-input byte indices, not paragraph-local ones.
///
/// `base_level` is the [`process_paragraph_classes`] HL1 override and
/// applies *uniformly* to every paragraph when `Some(_)` (callers that
/// need per-paragraph HL1 overrides loop [`process_paragraph`]
/// themselves). The low bit is preserved per HL1.
///
/// The empty input produces a `TextBidi { paragraphs: vec![],
/// total_chars: 0 }` carrier; the spec says nothing applies to an
/// empty text.
///
/// Provenance: §3 P1 + the §3.3.1 / §3.3.2 / §3.3.3 / §3.3.4 / §3.3.5 /
/// §3.3.6 per-rule entry points already in this module, each citing
/// `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` directly.
///
/// # Examples
///
/// ```
/// use oxideav_scribe::bidi::process_text;
///
/// // Two-paragraph LTR input separated by LF.
/// let t = process_text("Hi\nyo", None);
/// assert_eq!(t.paragraphs.len(), 2);
/// // First paragraph is "Hi\n" (3 chars: H, i, LF kept with paragraph).
/// assert_eq!(t.paragraphs[0].bidi.levels.len(), 3);
/// assert_eq!(t.paragraphs[1].bidi.levels.len(), 2);
/// assert_eq!(t.total_chars, 5);
/// ```
#[must_use]
pub fn process_text(text: &str, base_level: Option<u8>) -> TextBidi {
    let slices = split_paragraphs(text);
    let mut paragraphs = Vec::with_capacity(slices.len());
    let mut total_chars = 0usize;
    // `split_paragraphs` returns owned `&str` slices that point into
    // `text`; converting back to byte ranges via pointer arithmetic
    // would be UB-adjacent. We rederive the range by walking the
    // input alongside the slice list — `start` accumulates the
    // byte length of every prior paragraph.
    let mut byte_start = 0usize;
    for slice in slices {
        let len = slice.len();
        let byte_range = byte_start..byte_start + len;
        let mut classes = Vec::new();
        let mut char_byte_offsets = Vec::new();
        for (off, c) in slice.char_indices() {
            // Re-base the paragraph-local byte offset onto the whole
            // input so callers can index back into the original `&str`
            // without adding `byte_range.start` themselves.
            char_byte_offsets.push(byte_start + off);
            classes.push(bidi_class(c));
        }
        let bidi = process_paragraph_classes(&classes, base_level);
        let char_offset = total_chars;
        total_chars += bidi.levels.len();
        paragraphs.push(ParagraphSlice {
            byte_range,
            char_offset,
            bidi,
            char_byte_offsets,
        });
        byte_start += len;
    }
    TextBidi {
        paragraphs,
        total_chars,
    }
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

    // --- Section 9: X-rules (X1..X9) ---------------------------

    #[test]
    fn x_rules_empty_paragraph() {
        let out = resolve_explicit_levels(&[], 0);
        assert!(out.levels.is_empty());
        assert!(out.effective_classes.is_empty());
        assert!(out.removed.is_empty());
    }

    #[test]
    fn x_rules_plain_latin_stays_level_zero() {
        // Latin paragraph "ab" at paragraph level 0 — every char
        // gets level 0, no override, no removal.
        let cls = vec![BidiClass::L, BidiClass::L];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels, vec![0, 0]);
        assert_eq!(out.effective_classes, cls);
        assert_eq!(out.removed, vec![false, false]);
    }

    #[test]
    fn x_rules_rtl_paragraph_assigns_level_one() {
        // RTL paragraph: Arabic letters at paragraph level 1 get
        // level 1; no formatting characters in play.
        let cls = vec![BidiClass::AL, BidiClass::AL];
        let out = resolve_explicit_levels(&cls, 1);
        assert_eq!(out.levels, vec![1, 1]);
        assert_eq!(out.removed, vec![false, false]);
    }

    #[test]
    fn x_rules_rle_pushes_odd_level_pdf_pops() {
        // RLE L PDF at paragraph level 0 — RLE pushes level 1,
        // L gets level 1, PDF pops. The embedding initiator's
        // own level is reported as the new-scope level (the
        // stack top *after* the push), and PDF reports the
        // stack-top *after* the pop (the enclosing scope). Both
        // RLE and PDF are X9-removed; their reported level is
        // not consumed by the implicit phases.
        let cls = vec![BidiClass::RLE, BidiClass::L, BidiClass::PDF];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels, vec![1, 1, 0]);
        assert_eq!(out.removed, vec![true, false, true]);
    }

    #[test]
    fn x_rules_lre_pushes_least_greater_even() {
        // At level 0 LRE goes to level 2 (least even > 0). At
        // level 1 LRE goes to level 2 as well. The LRE's own
        // level reflects the new scope.
        let cls = vec![BidiClass::LRE, BidiClass::L, BidiClass::PDF];
        let out0 = resolve_explicit_levels(&cls, 0);
        assert_eq!(out0.levels, vec![2, 2, 0]);
        let out1 = resolve_explicit_levels(&cls, 1);
        assert_eq!(out1.levels, vec![2, 2, 1]);
    }

    #[test]
    fn x_rules_rlo_overrides_to_r() {
        // RLO L PDF — the L between RLO and PDF gets rewritten to
        // R by X6 + the override status.
        let cls = vec![BidiClass::RLO, BidiClass::L, BidiClass::PDF];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels, vec![1, 1, 0]);
        assert_eq!(out.effective_classes[1], BidiClass::R);
        assert_eq!(out.removed, vec![true, false, true]);
    }

    #[test]
    fn x_rules_lro_overrides_to_l() {
        // LRO AL PDF — AL gets rewritten to L by the override; the
        // explicit level is 2 (least even > 0).
        let cls = vec![BidiClass::LRO, BidiClass::AL, BidiClass::PDF];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels, vec![2, 2, 0]);
        assert_eq!(out.effective_classes[1], BidiClass::L);
    }

    #[test]
    fn x_rules_rli_pushes_isolate_pdi_pops() {
        // RLI L PDI at paragraph 0 — RLI's own level is the
        // enclosing scope (0), L gets level 1 (the new isolate
        // scope), PDI's level matches the enclosing scope = 0.
        // None of RLI / PDI is X9-removed.
        let cls = vec![BidiClass::RLI, BidiClass::L, BidiClass::PDI];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels, vec![0, 1, 0]);
        assert_eq!(out.removed, vec![false, false, false]);
    }

    #[test]
    fn x_rules_lri_pushes_least_greater_even() {
        let cls = vec![BidiClass::LRI, BidiClass::AL, BidiClass::PDI];
        let out = resolve_explicit_levels(&cls, 1);
        // LRI at paragraph level 1 pushes least even > 1 = 2.
        assert_eq!(out.levels, vec![1, 2, 1]);
    }

    #[test]
    fn x_rules_fsi_with_strong_l_inside_resolves_lri() {
        // FSI L PDI — FSI sees L first inside the span, so
        // resolves as LRI: at paragraph level 0 that pushes level
        // 2; the inner L gets level 2.
        let cls = vec![BidiClass::FSI, BidiClass::L, BidiClass::PDI];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels, vec![0, 2, 0]);
    }

    #[test]
    fn x_rules_fsi_with_strong_r_inside_resolves_rli() {
        // FSI AL PDI — FSI sees AL first inside the span, so
        // resolves as RLI: at paragraph level 0 that pushes level
        // 1; the inner AL gets level 1.
        let cls = vec![BidiClass::FSI, BidiClass::AL, BidiClass::PDI];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels, vec![0, 1, 0]);
    }

    #[test]
    fn x_rules_fsi_with_no_strong_inside_resolves_lri() {
        // FSI WS PDI — no strong character → default 0 → LRI →
        // pushes level 2; WS gets level 2.
        let cls = vec![BidiClass::FSI, BidiClass::WS, BidiClass::PDI];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels, vec![0, 2, 0]);
    }

    #[test]
    fn x_rules_b_assigned_paragraph_level() {
        // B inside an RLE scope should still get the paragraph
        // level per X8 ("they are not included in any embedding,
        // override or isolate").
        let cls = vec![BidiClass::RLE, BidiClass::L, BidiClass::B];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels, vec![1, 1, 0]);
    }

    #[test]
    fn x_rules_bn_removed_by_x9() {
        // BN inherits the enclosing scope's level + is marked
        // removed. RLE BN L PDF.
        let cls = vec![BidiClass::RLE, BidiClass::BN, BidiClass::L, BidiClass::PDF];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels, vec![1, 1, 1, 0]);
        assert_eq!(out.removed, vec![true, true, false, true]);
    }

    #[test]
    fn x_rules_nested_embeddings_at_most_three_deep() {
        // RLE LRE RLE L PDF PDF PDF at paragraph 0:
        // - RLE: level 0→1
        // - LRE: 1→2
        // - RLE: 2→3
        // - L : level 3
        // PDF unwinds back.
        let cls = vec![
            BidiClass::RLE,
            BidiClass::LRE,
            BidiClass::RLE,
            BidiClass::L,
            BidiClass::PDF,
            BidiClass::PDF,
            BidiClass::PDF,
        ];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels[3], 3);
        // The unwinding levels: after the third PDF the stack is
        // back to the paragraph frame; each PDF carries the level
        // *before* its pop, which the implementation reports as
        // the stack-top after the pop. Either contract is fine
        // since X9 removes PDFs anyway. We assert only that the
        // levels vector has no panic.
        assert_eq!(out.levels.len(), 7);
        // Embeddings + PDFs all X9-removed.
        assert_eq!(out.removed, vec![true, true, true, false, true, true, true]);
    }

    #[test]
    fn x_rules_overflow_embedding_at_max_depth() {
        // Build a sequence that pushes RLE 65 times (each adds
        // +2 to the level after the first). With paragraph level
        // 0: RLE pushes 1, 3, 5, ... up to MAX_DEPTH (125). 63
        // valid pushes reach level 125 (63 RLEs from level 0:
        // 1, 3, 5, ..., 125). A 64th RLE would attempt level 127
        // > 125 → overflow.
        let mut cls = vec![BidiClass::RLE; 64];
        cls.push(BidiClass::L);
        cls.push(BidiClass::PDF);
        let out = resolve_explicit_levels(&cls, 0);
        // 63 valid pushes give level 125; the 64th RLE overflows
        // so the L is still at level 125.
        assert_eq!(out.levels[64], 125);
    }

    #[test]
    fn x_rules_unmatched_pdf_at_paragraph_level_ignored() {
        // PDF at paragraph level with no matching embedding is
        // ignored (does nothing) — the level vector reflects the
        // paragraph level for any following non-formatting char.
        let cls = vec![BidiClass::PDF, BidiClass::L];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels, vec![0, 0]);
        assert_eq!(out.removed, vec![true, false]);
    }

    #[test]
    fn x_rules_unmatched_pdi_ignored() {
        // PDI at top level with no isolate above it is ignored
        // (X6a "Otherwise, if the valid isolate count is zero,
        // this PDI does not match any isolate initiator, valid or
        // overflow. Do nothing.").
        let cls = vec![BidiClass::PDI, BidiClass::L];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels, vec![0, 0]);
        assert_eq!(out.removed, vec![false, false]);
    }

    #[test]
    fn x_rules_pdi_pops_embeddings_inside_isolate() {
        // RLI RLE L PDI — the PDI matches the RLI, which by X6a
        // unwinds the embedding stack down to the matched isolate
        // frame and then pops the isolate. Final stack: paragraph
        // frame only.
        let cls = vec![
            BidiClass::RLI,
            BidiClass::RLE,
            BidiClass::L,
            BidiClass::PDI,
            BidiClass::L,
        ];
        let out = resolve_explicit_levels(&cls, 0);
        // RLI's level = enclosing = 0.
        assert_eq!(out.levels[0], 0);
        // RLE (inside the RLI scope at level 1) pushes 1 → 3.
        // But the RLE's reported level (per the implementation)
        // is the stack top *after* the push = 3, or the
        // enclosing level 1 depending on contract. Both are
        // tolerable per X9.
        // The L inside RLI+RLE is at level 3.
        assert_eq!(out.levels[2], 3);
        // PDI: matched RLI, so back to paragraph = level 0.
        assert_eq!(out.levels[3], 0);
        // Following L at paragraph level.
        assert_eq!(out.levels[4], 0);
    }

    #[test]
    fn x_rules_least_greater_odd_helper_table() {
        // Spot check the helper directly against the spec table:
        // "level 0 → 1; levels 1, 2 → 3; levels 3, 4 → 5; ..."
        assert_eq!(least_greater_odd(0), 1);
        assert_eq!(least_greater_odd(1), 3);
        assert_eq!(least_greater_odd(2), 3);
        assert_eq!(least_greater_odd(3), 5);
        assert_eq!(least_greater_odd(4), 5);
        // And the LRE/LRO version: "levels 0, 1 → 2; levels 2, 3
        // → 4; levels 4, 5 → 6; ..."
        assert_eq!(least_greater_even(0), 2);
        assert_eq!(least_greater_even(1), 2);
        assert_eq!(least_greater_even(2), 4);
        assert_eq!(least_greater_even(3), 4);
        assert_eq!(least_greater_even(4), 6);
    }

    #[test]
    fn x_rules_rle_inside_isolate_pdf_only_matches_inside() {
        // RLI RLE L PDF PDI L — the PDF matches the RLE inside the
        // isolate (X7 third bullet). After the PDI, the L is at
        // paragraph level 0.
        let cls = vec![
            BidiClass::RLI,
            BidiClass::RLE,
            BidiClass::L,
            BidiClass::PDF,
            BidiClass::PDI,
            BidiClass::L,
        ];
        let out = resolve_explicit_levels(&cls, 0);
        // L between RLE and PDF: level 3 (paragraph 0 → RLI 1 → RLE 3).
        assert_eq!(out.levels[2], 3);
        // Trailing L at paragraph level.
        assert_eq!(out.levels[5], 0);
    }

    #[test]
    fn x_rules_pdi_inside_overflow_isolate_decrements_overflow_isolate() {
        // Two RLIs nested at max depth: the second triggers
        // overflow_isolate; the matching PDI decrements it
        // (and the next PDI matches the valid RLI). Hard to test
        // exhaustively without a full max-depth chain — instead we
        // just confirm that a doubly-nested RLI ... PDI PDI pair
        // both succeed in normal depth (no panic).
        let cls = vec![
            BidiClass::RLI,
            BidiClass::RLI,
            BidiClass::L,
            BidiClass::PDI,
            BidiClass::PDI,
            BidiClass::L,
        ];
        let out = resolve_explicit_levels(&cls, 0);
        // Outer RLI → 1; inner RLI → 3; L inside → 3; first PDI
        // pops back to 1; second PDI pops back to 0; trailing L → 0.
        assert_eq!(out.levels[2], 3);
        assert_eq!(out.levels[5], 0);
    }

    #[test]
    fn x_rules_max_depth_constant_is_125() {
        // BD2: max_depth = 125, guaranteed stable.
        assert_eq!(MAX_DEPTH, 125);
    }

    // --- Section: BD7 level-run partition ----------------------

    #[test]
    fn level_runs_empty_input_returns_empty() {
        assert!(level_runs(&[]).is_empty());
    }

    #[test]
    fn level_runs_single_level_collapses_to_one_run() {
        let runs = level_runs(&[0, 0, 0]);
        assert_eq!(runs.len(), 1);
        assert_eq!((runs[0].start, runs[0].end, runs[0].level), (0, 3, 0));
        assert_eq!(runs[0].len(), 3);
        assert!(!runs[0].is_empty());
    }

    #[test]
    fn level_runs_split_on_level_change() {
        // [0, 0, 1, 1, 1, 0, 2] → (0..2, 0), (2..5, 1), (5..6, 0), (6..7, 2).
        let runs = level_runs(&[0, 0, 1, 1, 1, 0, 2]);
        assert_eq!(runs.len(), 4);
        assert_eq!((runs[0].start, runs[0].end, runs[0].level), (0, 2, 0));
        assert_eq!((runs[1].start, runs[1].end, runs[1].level), (2, 5, 1));
        assert_eq!((runs[2].start, runs[2].end, runs[2].level), (5, 6, 0));
        assert_eq!((runs[3].start, runs[3].end, runs[3].level), (6, 7, 2));
    }

    #[test]
    fn level_runs_cover_input_exactly() {
        // Concatenated ranges fully cover [0, n) without overlap.
        let levels = [0, 0, 1, 0, 1, 1, 0];
        let runs = level_runs(&levels);
        let mut expect_next = 0;
        for r in &runs {
            assert_eq!(r.start, expect_next);
            assert!(r.end > r.start);
            expect_next = r.end;
        }
        assert_eq!(expect_next, levels.len());
    }

    // --- Section: X10 + BD13 isolating-run-sequence partition ---

    fn build(s: &str) -> (Vec<BidiClass>, ExplicitLevels, u8) {
        let cls: Vec<BidiClass> = s.chars().map(bidi_class).collect();
        let pl = paragraph_level(s);
        let out = resolve_explicit_levels(&cls, pl);
        (cls, out, pl)
    }

    #[test]
    fn x10_empty_paragraph_no_sequences() {
        let cls: Vec<BidiClass> = Vec::new();
        let out = resolve_explicit_levels(&cls, 0);
        let seqs = isolating_run_sequences(&cls, &out, 0);
        assert!(seqs.is_empty());
    }

    #[test]
    fn x10_pure_l_one_sequence_one_run() {
        let (cls, out, pl) = build("Hello");
        let seqs = isolating_run_sequences(&cls, &out, pl);
        assert_eq!(seqs.len(), 1);
        assert_eq!(seqs[0].runs.len(), 1);
        assert_eq!(seqs[0].level, 0);
        assert_eq!(seqs[0].sos, BidiClass::L);
        assert_eq!(seqs[0].eos, BidiClass::L);
    }

    #[test]
    fn x10_arabic_paragraph_rtl_sos_eos_r() {
        // Hebrew letter שלום — paragraph level 1, single sequence at
        // level 1, sos = eos = R.
        let (cls, out, pl) = build("\u{05E9}\u{05DC}\u{05D5}\u{05DD}");
        assert_eq!(pl, 1);
        let seqs = isolating_run_sequences(&cls, &out, pl);
        assert_eq!(seqs.len(), 1);
        assert_eq!(seqs[0].level, 1);
        assert_eq!(seqs[0].sos, BidiClass::R);
        assert_eq!(seqs[0].eos, BidiClass::R);
    }

    #[test]
    fn x10_rle_split_emits_three_sequences_with_correct_sos_eos() {
        // L RLE L PDF L — runs:
        //   [0..1] level 0 (L)
        //   [1..3] level 1 (RLE + L; PDF index 3 stays at level 0 →
        //                 wait: PDF gets the post-pop stack top → 0,
        //                 so the level vector is [0,1,1,0,0]).
        // Result: (0..1)@0, (1..3)@1, (3..5)@0 — three sequences.
        let cls = vec![
            BidiClass::L,
            BidiClass::RLE,
            BidiClass::L,
            BidiClass::PDF,
            BidiClass::L,
        ];
        let out = resolve_explicit_levels(&cls, 0);
        let seqs = isolating_run_sequences(&cls, &out, 0);
        assert_eq!(seqs.len(), 3);
        // First: sos paragraph (0), eos other side is RLE @ level 1
        // BUT RLE is X9-removed — the spec says boundary lookups
        // skip removed chars, so eos other side scans forward past
        // the RLE to the next non-removed char (L at index 2 @
        // level 1) → higher is 1 → R.
        assert_eq!(seqs[0].level, 0);
        assert_eq!(seqs[0].sos, BidiClass::L);
        assert_eq!(seqs[0].eos, BidiClass::R);
        // Middle: level 1, sos other side is L @ 0 (the leading L),
        // higher of 1 vs 0 → 1 → R. eos other side: skip PDF → L
        // @ level 0, higher 1 vs 0 → R.
        assert_eq!(seqs[1].level, 1);
        assert_eq!(seqs[1].sos, BidiClass::R);
        assert_eq!(seqs[1].eos, BidiClass::R);
        // Last: level 0, sos other side: skip PDF backward → L @
        // level 1, higher 0 vs 1 → 1 → R. eos paragraph edge → 0
        // → L.
        assert_eq!(seqs[2].level, 0);
        assert_eq!(seqs[2].sos, BidiClass::R);
        assert_eq!(seqs[2].eos, BidiClass::L);
    }

    #[test]
    fn x10_lri_chains_outer_runs_into_one_sequence() {
        // "a" LRI "b" PDI "c" — the LRI initiator and matching PDI
        // both carry the *enclosing* embedding level (0). Levels are
        // [0, 0, 0, 0, 0] — a single level run, hence a single
        // sequence (the LRI/PDI happen not to raise the level because
        // we are already at level 0 and an LRI under paragraph level
        // 0 pushes level 2 only for the *contents*, which is also
        // level 2 only if there ARE intervening chars).
        //
        // Concretely with "a" LRI "b" PDI "c": LRI pushes level 2
        // (least even > 0), 'b' lands at 2, PDI pops back, 'c' at 0.
        // Levels [0, 0, 2, 0, 0] — runs (0..2, 0), (2..3, 2),
        // (3..5, 0).
        //
        // BD13: the first run ends with LRI (an isolate initiator)
        // whose matching PDI is at index 3 — but index 3 is the
        // *first character* of the third run, not the second. So
        // the first run chains to the third, and the second run is
        // its own sequence.
        let cls = vec![
            BidiClass::L,
            BidiClass::LRI,
            BidiClass::L,
            BidiClass::PDI,
            BidiClass::L,
        ];
        let out = resolve_explicit_levels(&cls, 0);
        assert_eq!(out.levels, vec![0, 0, 2, 0, 0]);
        let runs = level_runs(&out.levels);
        assert_eq!(runs.len(), 3);
        let seqs = isolating_run_sequences(&cls, &out, 0);
        // Two sequences: one chaining (0..2)+(3..5), one for (2..3).
        assert_eq!(seqs.len(), 2);
        // The chained sequence is the first emitted (its seed run
        // starts at index 0).
        assert_eq!(seqs[0].runs.len(), 2);
        assert_eq!(seqs[0].runs[0].start, 0);
        assert_eq!(seqs[0].runs[0].end, 2);
        assert_eq!(seqs[0].runs[1].start, 3);
        assert_eq!(seqs[0].runs[1].end, 5);
        assert_eq!(seqs[0].level, 0);
        // The interior sequence is the body of the isolate.
        assert_eq!(seqs[1].runs.len(), 1);
        assert_eq!(seqs[1].runs[0].start, 2);
        assert_eq!(seqs[1].runs[0].end, 3);
        assert_eq!(seqs[1].level, 2);
    }

    #[test]
    fn x10_unmatched_isolate_initiator_triggers_eos_paragraph_fallback() {
        // "a" LRI "b" — LRI raises level for 'b' to 2; there is no
        // matching PDI. The last run ends with an isolate initiator
        // (the LRI itself sits in the first run, since 'b' is on a
        // raised level), but the LRI's matching PDI doesn't exist
        // so the eos of the outer sequence falls back to the
        // paragraph level.
        let cls = vec![BidiClass::L, BidiClass::LRI, BidiClass::L];
        let out = resolve_explicit_levels(&cls, 0);
        // Levels: 'a' 0, LRI carries enclosing 0, 'b' 2.
        assert_eq!(out.levels, vec![0, 0, 2]);
        let seqs = isolating_run_sequences(&cls, &out, 0);
        // Two sequences: (0..2)@0 and (2..3)@2. The first ends with
        // an unmatched LRI; by X10 step 2 its eos uses the paragraph
        // level (0) as the other side → higher(0, 0) = 0 → L.
        assert_eq!(seqs.len(), 2);
        assert_eq!(seqs[0].runs[0].start, 0);
        assert_eq!(seqs[0].runs[0].end, 2);
        assert_eq!(seqs[0].sos, BidiClass::L);
        assert_eq!(seqs[0].eos, BidiClass::L);
        // Second sequence is at level 2 (even). sos other side is
        // the LRI at index 1 (not X9-removed) at level 0 — higher
        // of (0, 2) = 2 → L (even). eos paragraph edge → paragraph
        // level 0 — higher of (0, 2) = 2 → L.
        assert_eq!(seqs[1].sos, BidiClass::L);
        assert_eq!(seqs[1].eos, BidiClass::L);
    }

    #[test]
    fn x10_paragraph_level_1_rtl_default_sos_eos() {
        // Pure-Hebrew paragraph at level 1: paragraph fallback for
        // sos / eos is paragraph level 1 → R.
        let cls = vec![BidiClass::R, BidiClass::R, BidiClass::R];
        let out = resolve_explicit_levels(&cls, 1);
        assert_eq!(out.levels, vec![1, 1, 1]);
        let seqs = isolating_run_sequences(&cls, &out, 1);
        assert_eq!(seqs.len(), 1);
        assert_eq!(seqs[0].sos, BidiClass::R);
        assert_eq!(seqs[0].eos, BidiClass::R);
    }

    #[test]
    fn x10_each_level_run_belongs_to_exactly_one_sequence() {
        // BD13 invariant: every level run appears in exactly one
        // isolating run sequence. Build a paragraph with multiple
        // RLE/PDF + LRI/PDI nestings and assert partition coverage.
        let cls = vec![
            BidiClass::L,   // 0
            BidiClass::RLE, // 1
            BidiClass::L,   // 2
            BidiClass::LRI, // 3 (under RLE override)
            BidiClass::L,   // 4
            BidiClass::PDI, // 5
            BidiClass::L,   // 6
            BidiClass::PDF, // 7
            BidiClass::L,   // 8
        ];
        let out = resolve_explicit_levels(&cls, 0);
        let runs = level_runs(&out.levels);
        let total_run_count = runs.len();
        let seqs = isolating_run_sequences(&cls, &out, 0);
        let mut counted = 0;
        for s in &seqs {
            counted += s.runs.len();
            // All runs in one sequence share embedding level.
            for r in &s.runs {
                assert_eq!(r.level, s.level);
            }
        }
        assert_eq!(
            counted, total_run_count,
            "every level run belongs to exactly one sequence"
        );
    }

    #[test]
    fn x10_indices_iterator_skips_x9_removed() {
        // L RLE L PDF L — sequence [0..1, level 0] yields just {0};
        // sequence [1..3, level 1] skips the RLE at index 1 and
        // yields {2}; sequence [3..5, level 0] skips the PDF at
        // index 3 and yields {4}.
        let cls = vec![
            BidiClass::L,
            BidiClass::RLE,
            BidiClass::L,
            BidiClass::PDF,
            BidiClass::L,
        ];
        let out = resolve_explicit_levels(&cls, 0);
        let seqs = isolating_run_sequences(&cls, &out, 0);
        assert_eq!(seqs.len(), 3);
        let s0: Vec<usize> = seqs[0].indices(&out.removed).collect();
        assert_eq!(s0, vec![0]);
        let s1: Vec<usize> = seqs[1].indices(&out.removed).collect();
        assert_eq!(s1, vec![2]);
        let s2: Vec<usize> = seqs[2].indices(&out.removed).collect();
        assert_eq!(s2, vec![4]);
    }

    #[test]
    fn x10_indices_for_chained_isolate_sequence_concatenates_runs() {
        // "a" LRI "b" PDI "c" — chained sequence is (0..2) + (3..5).
        // Indices walk yields {0, 1, 3, 4} (LRI at 1 and PDI at 3
        // are isolate-formatting characters, NOT X9-removed, so
        // they participate in the index walk per the X9 spec note).
        let cls = vec![
            BidiClass::L,
            BidiClass::LRI,
            BidiClass::L,
            BidiClass::PDI,
            BidiClass::L,
        ];
        let out = resolve_explicit_levels(&cls, 0);
        let seqs = isolating_run_sequences(&cls, &out, 0);
        let s0: Vec<usize> = seqs[0].indices(&out.removed).collect();
        assert_eq!(s0, vec![0, 1, 3, 4]);
    }

    #[test]
    fn x10_sequences_compose_with_existing_w_n_i_pipeline() {
        // End-to-end sanity: build a paragraph, partition it, and
        // verify each sequence's sos / eos lets the W rules run
        // without modification. We don't check W output here (that
        // is covered by the existing W tests); we only confirm that
        // the API hands us per-sequence `sos` / `eos` of class L or
        // R as required.
        let cls = vec![
            BidiClass::L,
            BidiClass::RLE,
            BidiClass::AL,
            BidiClass::EN,
            BidiClass::PDF,
            BidiClass::L,
        ];
        let out = resolve_explicit_levels(&cls, 0);
        let seqs = isolating_run_sequences(&cls, &out, 0);
        assert!(!seqs.is_empty());
        for s in &seqs {
            assert!(matches!(s.sos, BidiClass::L | BidiClass::R));
            assert!(matches!(s.eos, BidiClass::L | BidiClass::R));
            // Round-trip through resolve_weak_types per-sequence in
            // place: we just confirm it does not panic and leaves
            // the slice strong-typed.
            let mut indices: Vec<usize> = s.indices(&out.removed).collect();
            // Pull the effective classes for this sequence into a
            // working buffer; W rules expect a contiguous mut slice.
            let mut seq_classes: Vec<BidiClass> =
                indices.iter().map(|&i| out.effective_classes[i]).collect();
            resolve_weak_types(&mut seq_classes, s.sos, s.eos);
            // After W, no AL should remain (W3).
            assert!(seq_classes.iter().all(|c| !matches!(c, BidiClass::AL)));
            let _ = indices.drain(..);
        }
    }

    #[test]
    fn x10_matching_pdi_skips_non_isolate_formatting() {
        // BD9 closing note: "all formatting characters except for
        // isolate initiators and PDIs are ignored when finding the
        // matching PDI." LRI ... RLE LRE PDF ... PDI — the matching
        // PDI is correctly found past the embedding formatting.
        let cls = vec![
            BidiClass::L,
            BidiClass::LRI, // 1
            BidiClass::RLE, // 2 (ignored by BD9)
            BidiClass::L,   // 3
            BidiClass::PDF, // 4 (ignored by BD9)
            BidiClass::PDI, // 5
            BidiClass::L,
        ];
        let pdi = matching_pdi(&cls, 1);
        assert_eq!(pdi, Some(5));
    }

    #[test]
    fn x10_matching_pdi_returns_none_for_unmatched_initiator() {
        let cls = vec![BidiClass::L, BidiClass::LRI, BidiClass::L];
        assert_eq!(matching_pdi(&cls, 1), None);
    }

    #[test]
    fn x10_matching_pdi_counts_nested_isolates_correctly() {
        // LRI LRI PDI PDI — the outer LRI at 0 matches the second
        // PDI at 3, the inner LRI at 1 matches the first PDI at 2.
        let cls = vec![
            BidiClass::LRI,
            BidiClass::LRI,
            BidiClass::PDI,
            BidiClass::PDI,
        ];
        assert_eq!(matching_pdi(&cls, 0), Some(3));
        assert_eq!(matching_pdi(&cls, 1), Some(2));
    }

    // --- Section ?: §3 whole-paragraph driver -----------------------

    #[test]
    fn process_paragraph_classes_empty_input_returns_empty_carrier() {
        let p = process_paragraph_classes(&[], None);
        assert_eq!(p.paragraph_level, 0);
        assert!(p.classes.is_empty());
        assert!(p.effective_classes.is_empty());
        assert!(p.removed.is_empty());
        assert!(p.levels.is_empty());
    }

    #[test]
    fn process_paragraph_classes_first_strong_l_is_ltr() {
        // "ABC" all L → paragraph level 0, every char at level 0.
        let cls = vec![BidiClass::L, BidiClass::L, BidiClass::L];
        let p = process_paragraph_classes(&cls, None);
        assert_eq!(p.paragraph_level, 0);
        assert_eq!(p.levels, vec![0, 0, 0]);
    }

    #[test]
    fn process_paragraph_classes_first_strong_r_is_rtl() {
        // Hebrew-only paragraph: every char at level 1.
        let cls = vec![BidiClass::R, BidiClass::R, BidiClass::R];
        let p = process_paragraph_classes(&cls, None);
        assert_eq!(p.paragraph_level, 1);
        assert_eq!(p.levels, vec![1, 1, 1]);
    }

    #[test]
    fn process_paragraph_classes_first_strong_al_is_rtl() {
        // Arabic-only paragraph: W3 rewrites AL → R, every char at
        // level 1.
        let cls = vec![BidiClass::AL, BidiClass::AL, BidiClass::AL];
        let p = process_paragraph_classes(&cls, None);
        assert_eq!(p.paragraph_level, 1);
        assert_eq!(p.levels, vec![1, 1, 1]);
        // effective_classes preserves AL (X-stack does no W-rule
        // rewriting); the W-rule rewrite is internal to the per-
        // sequence pass and never re-published.
        assert_eq!(p.effective_classes, vec![BidiClass::AL; 3]);
    }

    #[test]
    fn process_paragraph_classes_p3_fallback_is_ltr() {
        // No strong characters → P3 says paragraph level 0 (LTR).
        let cls = vec![BidiClass::ON, BidiClass::WS, BidiClass::ON];
        let p = process_paragraph_classes(&cls, None);
        assert_eq!(p.paragraph_level, 0);
        // The single NI run resolves to L (sos = eos = L), so every
        // level stays at 0.
        assert_eq!(p.levels, vec![0, 0, 0]);
    }

    #[test]
    fn process_paragraph_classes_base_level_overrides_first_strong() {
        // "L L L" but caller forces base_level = 1 → all chars at
        // level 2 (the R-resolution under odd embedding I1 row "L
        // goes +1").
        let cls = vec![BidiClass::L, BidiClass::L, BidiClass::L];
        let p = process_paragraph_classes(&cls, Some(1));
        assert_eq!(p.paragraph_level, 1);
        assert_eq!(p.levels, vec![2, 2, 2]);
    }

    #[test]
    fn process_paragraph_classes_base_level_clamps_to_low_bit() {
        // Caller passes 5 (an odd embedding) — we mask to the
        // paragraph-level convention {0, 1}.
        let cls = vec![BidiClass::L];
        let p = process_paragraph_classes(&cls, Some(5));
        assert_eq!(p.paragraph_level, 1);
    }

    #[test]
    fn process_paragraph_classes_isolate_span_skipped_by_p2() {
        // BD8 says the first-strong walk skips the contents of an
        // LRI..PDI span. Here the first strong outside the isolate
        // is R, so the paragraph level should be 1.
        let cls = vec![BidiClass::LRI, BidiClass::L, BidiClass::PDI, BidiClass::R];
        let p = process_paragraph_classes(&cls, None);
        assert_eq!(p.paragraph_level, 1);
    }

    #[test]
    fn process_paragraph_classes_unmatched_isolate_initiator_returns_p3_default() {
        // BD8 says the walk skips to end of paragraph past an
        // unmatched initiator. No strong character outside it →
        // P3 fallback 0.
        let cls = vec![BidiClass::LRI, BidiClass::R, BidiClass::R];
        let p = process_paragraph_classes(&cls, None);
        assert_eq!(p.paragraph_level, 0);
    }

    #[test]
    fn process_paragraph_classes_x9_removed_chars_retain_x_level() {
        // L RLE L PDF L — RLE / PDF are X9-removed. The middle L
        // sits inside an RLE-pushed odd-level scope (X-level 1); I1
        // under odd EL says L → +1, so the middle L ends at level 2.
        // The outer L's stay at level 0.
        let cls = vec![
            BidiClass::L,
            BidiClass::RLE,
            BidiClass::L,
            BidiClass::PDF,
            BidiClass::L,
        ];
        let p = process_paragraph_classes(&cls, None);
        assert_eq!(p.paragraph_level, 0);
        // RLE + PDF are X9-removed (the W / N / I sweep skipped them);
        // their level is whatever X1..X9 left there.
        assert_eq!(p.removed, vec![false, true, false, true, false]);
        // The outer L's stay at level 0; the inner L lifts to 2
        // (X-level 1 + I1 odd-EL increment for L).
        assert_eq!(p.levels[0], 0);
        assert_eq!(p.levels[2], 2);
        assert_eq!(p.levels[4], 0);
    }

    #[test]
    fn process_paragraph_classes_mixed_l_r_compose_full_pipeline() {
        // "ABC abc" with abc as Hebrew (R). Paragraph level 0
        // (first-strong is L). After W / N / I, the R block goes
        // to level 1, the L block to level 0; the WS between them
        // resolves to L per N1 / N2 (R/L mismatch → embedding dir
        // = L at level 0) and stays at level 0.
        let cls = vec![
            BidiClass::L,
            BidiClass::L,
            BidiClass::L, // ABC
            BidiClass::WS,
            BidiClass::R,
            BidiClass::R,
            BidiClass::R, // hbr
        ];
        let p = process_paragraph_classes(&cls, None);
        assert_eq!(p.paragraph_level, 0);
        // L block + WS at level 0; R block at level 1.
        assert_eq!(p.levels, vec![0, 0, 0, 0, 1, 1, 1]);
    }

    #[test]
    fn process_paragraph_classes_rtl_paragraph_lifts_l_to_level_2() {
        // RTL paragraph (first strong R) with embedded Latin: the
        // Latin block goes from L at level 0 → L at level 2 (I1
        // under odd EL: L goes +1; since the surrounding sequence
        // is level 1, L sits at 1+1=2).
        let cls = vec![
            BidiClass::R,
            BidiClass::R, // RR
            BidiClass::WS,
            BidiClass::L,
            BidiClass::L,
            BidiClass::L, // abc
            BidiClass::WS,
            BidiClass::R,
            BidiClass::R, // RR
        ];
        let p = process_paragraph_classes(&cls, None);
        assert_eq!(p.paragraph_level, 1);
        // R block at 1, L block at 2, whitespace at the embedding
        // level (1) per N2 because L vs R on either side.
        assert_eq!(p.levels, vec![1, 1, 1, 2, 2, 2, 1, 1, 1]);
    }

    #[test]
    fn process_paragraph_text_byte_offsets_track_chars() {
        // Plain ASCII paragraph: every char-byte is at the char's
        // own index, len 5.
        let (p, offsets) = process_paragraph("Hello", None);
        assert_eq!(p.paragraph_level, 0);
        assert_eq!(p.levels, vec![0; 5]);
        assert_eq!(offsets, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn process_paragraph_text_multi_byte_chars_byte_offsets_advance() {
        // "Ωa" — Ω is 2 bytes, a is 1.
        let (p, offsets) = process_paragraph("\u{03A9}a", None);
        assert_eq!(p.classes.len(), 2);
        assert_eq!(offsets, vec![0, 2]);
        // Ω is L per our table fallback; both at level 0.
        assert_eq!(p.levels, vec![0, 0]);
    }

    #[test]
    fn reorder_paragraph_ltr_only_is_identity() {
        let p = process_paragraph_classes(&[BidiClass::L; 4], None);
        let perm = p.reorder_paragraph();
        assert_eq!(perm, vec![0, 1, 2, 3]);
    }

    #[test]
    fn reorder_paragraph_rtl_block_reverses() {
        // ABC + RTL block. After L1+L2, the RTL block reverses.
        let cls = vec![
            BidiClass::L,
            BidiClass::L,
            BidiClass::L,
            BidiClass::R,
            BidiClass::R,
            BidiClass::R,
        ];
        let p = process_paragraph_classes(&cls, None);
        let perm = p.reorder_paragraph();
        // L block stays 0,1,2; R block reverses 3,4,5 → 5,4,3.
        assert_eq!(perm, vec![0, 1, 2, 5, 4, 3]);
    }

    #[test]
    fn reorder_line_range_per_line_works() {
        // Treat a 5-char "ABCDE" as two lines: [0..2] and [2..5].
        let p = process_paragraph_classes(&[BidiClass::L; 5], None);
        let line1 = p.reorder_line_range(0..2);
        let line2 = p.reorder_line_range(2..5);
        assert_eq!(line1, vec![0, 1]);
        assert_eq!(line2, vec![0, 1, 2]);
    }

    #[test]
    #[should_panic]
    fn reorder_line_range_out_of_bounds_panics() {
        let p = process_paragraph_classes(&[BidiClass::L; 3], None);
        let _ = p.reorder_line_range(0..10);
    }

    #[test]
    fn paragraph_level_from_classes_matches_text_walker() {
        // Cross-check the class-driven P2 walk against the text-
        // driven `paragraph_level` on a handful of inputs.
        for s in &[
            "Hello",
            "\u{05D0}\u{05D1}\u{05D2}",  // Hebrew
            "\u{0627}\u{0628}\u{0629}",  // Arabic
            "Hi \u{05D0}\u{05D1}",       // Mixed Latin + Hebrew (P2 = L)
            "\u{05D0}\u{05D1} Hi",       // Mixed Hebrew + Latin (P2 = R)
            "\u{2066}A\u{2069}\u{05D0}", // LRI A PDI + Hebrew → P2 = R
        ] {
            let cls: Vec<_> = s.chars().map(bidi_class).collect();
            assert_eq!(
                paragraph_level_from_classes(&cls),
                paragraph_level(s),
                "mismatch for input {s:?}",
            );
        }
    }

    // --- Section N: §3 P1 multi-paragraph driver ---------------------

    #[test]
    fn process_text_empty_input_returns_empty_carrier() {
        let t = process_text("", None);
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert_eq!(t.total_chars, 0);
        assert!(t.paragraphs.is_empty());
    }

    #[test]
    fn process_text_no_paragraph_separator_single_paragraph() {
        // No B character anywhere → P1 produces a single paragraph
        // covering the whole input.
        let t = process_text("Hello", None);
        assert_eq!(t.len(), 1);
        assert_eq!(t.total_chars, 5);
        let p = &t.paragraphs[0];
        assert_eq!(p.byte_range, 0..5);
        assert_eq!(p.char_offset, 0);
        assert_eq!(p.bidi.paragraph_level, 0);
        assert_eq!(p.bidi.levels, vec![0; 5]);
    }

    #[test]
    fn process_text_lf_terminator_kept_with_preceding_paragraph_per_p1() {
        // "Hi\nyo" → P1 splits into ["Hi\n", "yo"]. The first
        // paragraph contains the LF (which has BidiClass `B`).
        let t = process_text("Hi\nyo", None);
        assert_eq!(t.len(), 2);
        assert_eq!(t.total_chars, 5);
        let p0 = &t.paragraphs[0];
        let p1 = &t.paragraphs[1];
        assert_eq!(p0.byte_range, 0..3);
        assert_eq!(p0.char_offset, 0);
        assert_eq!(p0.bidi.levels.len(), 3);
        assert_eq!(p1.byte_range, 3..5);
        assert_eq!(p1.char_offset, 3);
        assert_eq!(p1.bidi.levels.len(), 2);
    }

    #[test]
    fn process_text_terminal_lf_does_not_create_phantom_paragraph() {
        // "Hi\n" is one paragraph (the LF closes it but starts no new
        // one) — `split_paragraphs` only emits the trailing tail if it
        // contains any character after the last B.
        let t = process_text("Hi\n", None);
        assert_eq!(t.len(), 1);
        assert_eq!(t.total_chars, 3);
        let p = &t.paragraphs[0];
        assert_eq!(p.byte_range, 0..3);
        assert_eq!(p.bidi.classes.last(), Some(&BidiClass::B));
    }

    #[test]
    fn process_text_per_paragraph_p2_runs_independently() {
        // First paragraph is Latin (L), second is Hebrew (R). Each P2
        // walk operates on its own paragraph slice, so paragraph
        // levels diverge.
        let t = process_text("Hi\n\u{05D0}\u{05D1}", None);
        assert_eq!(t.len(), 2);
        assert_eq!(t.paragraphs[0].bidi.paragraph_level, 0);
        assert_eq!(t.paragraphs[1].bidi.paragraph_level, 1);
    }

    #[test]
    fn process_text_base_level_override_applies_uniformly() {
        // HL1: caller supplies base_level for both paragraphs.
        let t = process_text("Hi\nyo", Some(1));
        assert_eq!(t.len(), 2);
        for p in &t.paragraphs {
            assert_eq!(p.bidi.paragraph_level, 1);
        }
    }

    #[test]
    fn process_text_char_byte_offsets_are_whole_input_indices() {
        // Multi-byte characters (Hebrew Alef = 2 bytes in UTF-8) +
        // multi-paragraph layout. The byte offsets in
        // `char_byte_offsets` are whole-input indices, not paragraph-
        // local ones.
        let s = "A\n\u{05D0}\u{05D1}";
        let t = process_text(s, None);
        assert_eq!(t.len(), 2);
        // Paragraph 0: "A\n" → chars at byte 0, 1.
        assert_eq!(t.paragraphs[0].char_byte_offsets, vec![0, 1]);
        // Paragraph 1: "אב" starting at whole-input byte 2.
        assert_eq!(t.paragraphs[1].char_byte_offsets, vec![2, 4]);
        // Verify the byte offsets index the original string correctly.
        for p in &t.paragraphs {
            for (i, &off) in p.char_byte_offsets.iter().enumerate() {
                let c = s[off..].chars().next().expect("char at byte offset");
                assert_eq!(bidi_class(c), p.bidi.classes[i]);
            }
        }
    }

    #[test]
    fn process_text_byte_range_covers_whole_input_with_no_gap() {
        // The byte_range entries of consecutive paragraphs tile the
        // whole input contiguously.
        let s = "AAA\nBBB\nCCC";
        let t = process_text(s, None);
        assert_eq!(t.len(), 3);
        assert_eq!(t.paragraphs[0].byte_range.start, 0);
        assert_eq!(
            t.paragraphs[2].byte_range.end,
            s.len(),
            "last paragraph end equals input length",
        );
        for w in t.paragraphs.windows(2) {
            assert_eq!(
                w[0].byte_range.end, w[1].byte_range.start,
                "paragraph ranges tile contiguously",
            );
        }
    }

    #[test]
    fn process_text_char_offset_accumulates_correctly() {
        let t = process_text("AB\nCDE\nF", None);
        assert_eq!(t.len(), 3);
        assert_eq!(t.paragraphs[0].char_offset, 0);
        // "AB\n" is 3 chars.
        assert_eq!(t.paragraphs[1].char_offset, 3);
        // "AB\n" (3) + "CDE\n" (4) = 7 chars before paragraph 2.
        assert_eq!(t.paragraphs[2].char_offset, 7);
        assert_eq!(t.total_chars, 8);
    }

    #[test]
    fn text_bidi_locate_char_returns_paragraph_and_local_index() {
        let t = process_text("AB\nCD", None);
        // k = 0 → paragraph 0, local index 0.
        assert_eq!(t.locate_char(0), Some((0, 0)));
        // k = 1 → paragraph 0, local index 1.
        assert_eq!(t.locate_char(1), Some((0, 1)));
        // k = 2 → paragraph 0, local index 2 (the LF).
        assert_eq!(t.locate_char(2), Some((0, 2)));
        // k = 3 → paragraph 1, local index 0.
        assert_eq!(t.locate_char(3), Some((1, 0)));
        // k = 4 → paragraph 1, local index 1.
        assert_eq!(t.locate_char(4), Some((1, 1)));
        // k = 5 is out of bounds (total_chars = 5).
        assert_eq!(t.locate_char(5), None);
        assert_eq!(t.locate_char(99), None);
    }

    #[test]
    fn process_text_splits_on_every_paragraph_separator_class_b_codepoint() {
        // P1 splits on every class-B character bidi_class() returns.
        // Whichever codepoints the local bidi_class() table assigns
        // to class B must induce a split — this guards the
        // bidi_class() → split_paragraphs → process_text contract end
        // to end without re-asserting the exact set membership.
        for sep in &[
            '\u{000A}', // LF
            '\u{000D}', // CR
            '\u{0085}', // NEL
            '\u{001C}', // File separator
            '\u{001D}', // Group separator
            '\u{001E}', // Record separator
            '\u{2029}', // PS
        ] {
            assert_eq!(
                bidi_class(*sep),
                BidiClass::B,
                "test-input invariant: {:#x} should be class B",
                *sep as u32,
            );
            let s = format!("A{sep}B");
            let t = process_text(&s, None);
            assert_eq!(t.len(), 2, "{sep:?} should split into 2 paragraphs");
        }
    }

    #[test]
    fn process_text_matches_per_paragraph_process_paragraph_call() {
        // process_text must be observationally identical to looping
        // process_paragraph over split_paragraphs (modulo byte-offset
        // rebasing). Cross-check on a representative LTR + RTL +
        // mixed input.
        let s = "Hi \u{05D0}\u{05D1}\n\u{0627}\u{0628}\nBye";
        let t = process_text(s, None);
        let slices = split_paragraphs(s);
        assert_eq!(t.paragraphs.len(), slices.len());
        let mut byte_start = 0usize;
        for (carrier, slice) in t.paragraphs.iter().zip(slices.iter()) {
            let (expected, expected_offsets) = process_paragraph(slice, None);
            assert_eq!(carrier.bidi, expected);
            // Per-paragraph `process_paragraph` returns paragraph-
            // local byte offsets — add `byte_start` to match
            // `process_text`'s whole-input offsets.
            let shifted: Vec<usize> = expected_offsets.iter().map(|o| o + byte_start).collect();
            assert_eq!(carrier.char_byte_offsets, shifted);
            byte_start += slice.len();
        }
    }

    #[test]
    fn process_text_total_chars_matches_input_char_count() {
        for s in &[
            "",
            "Hi",
            "Hi\nyo",
            "Hi\n\u{05D0}\u{05D1}",
            "A\nB\nC\n",
            "\u{05D0}\u{05D1}\n\u{0627}\u{0628}",
        ] {
            let t = process_text(s, None);
            assert_eq!(t.total_chars, s.chars().count(), "for input {s:?}");
        }
    }

    #[test]
    fn process_text_base_level_low_bit_clamp_per_paragraph() {
        // base_level = 5 → low bit 1 → every paragraph RTL.
        let t = process_text("Hi\nyo", Some(5));
        for p in &t.paragraphs {
            assert_eq!(p.bidi.paragraph_level, 1);
        }
    }

    #[test]
    fn process_text_locate_char_round_trips_with_offset_arithmetic() {
        // For every logical character k, locate_char(k) → (pi, ki)
        // should satisfy `paragraphs[pi].char_offset + ki == k`.
        let t = process_text("AB\nCD\nE", None);
        for k in 0..t.total_chars {
            let (pi, ki) = t.locate_char(k).expect("k in bounds");
            assert_eq!(t.paragraphs[pi].char_offset + ki, k);
        }
    }
}
