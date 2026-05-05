//! Arabic / Hebrew RTL contextual joining (round 7).
//!
//! Implements the Unicode joining-type lookup + adjacency state machine
//! that picks one of `{Isol, Init, Medi, Fina}` for every character in
//! a run. Downstream the chosen form selects which OpenType GSUB feature
//! tag (`isol` / `init` / `medi` / `fina`) the shaper applies — letting
//! a font swap "isolated alif" for "final alif", etc.
//!
//! ## References
//!
//! - Unicode Standard Annex #44 (UCD) — `ArabicShaping.txt` (joining
//!   type per codepoint).
//! - Unicode core specification §9.2 — Arabic joining algorithm.
//! - Microsoft OpenType Layout — Arabic shaping (the `isol` / `init` /
//!   `medi` / `fina` feature contract).
//!
//! No HarfBuzz / FreeType / pango / ICU layout source consulted — this
//! is a clean-room implementation of the algorithm described in the
//! Unicode + OpenType specs.
//!
//! ## Algorithm
//!
//! 1. For each char compute its [`JoiningClass`] via [`joining_class`].
//! 2. Walk the run left-to-right, **skipping `T` (transparent)** chars
//!    when computing neighbours, to determine whether each non-T char
//!    can join with its left/right neighbour.
//! 3. The form is then:
//!    - `Isol` — neither side joins
//!    - `Init` — only the right side joins (left edge of a chain)
//!    - `Medi` — both sides join (interior of a chain)
//!    - `Fina` — only the left side joins (right edge of a chain)
//!
//! "Joins" here means: the neighbour's joining class is in
//! `{D, R, C}` for the *left* neighbour (it can join to its right) and
//! `{D, L, C}` for the *right* neighbour (it can join to its left).
//! `T` chars (combining marks, etc.) are pass-through — they inherit
//! the form of the char they decorate.
//!
//! Logical-order input is assumed (post-bidi). The state machine itself
//! is direction-agnostic — RTL display order is the rasterizer's
//! concern, not this module's.

#![allow(clippy::manual_range_contains)]

/// Unicode joining type. Names match the UCD `ArabicShaping.txt`
/// single-letter codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoiningClass {
    /// Non-joining. Breaks the joining chain on both sides. Default for
    /// non-Arabic / non-Syriac codepoints, plus Arabic punctuation and
    /// most digits.
    U,
    /// Left-joining. Joins to its left neighbour only. Rare in Arabic
    /// (used in some Manichaean / Phags-pa style scripts; included for
    /// completeness).
    L,
    /// Right-joining. Joins to its right neighbour only. The "alif"
    /// family: alif, dal, dhal, reh, zain, waw and a few others.
    R,
    /// Dual-joining. Joins on both sides. The bulk of the Arabic
    /// alphabet — ba, ta, tha, jeem, hah, etc.
    D,
    /// Joining-causing. Forces a joining context regardless of intrinsic
    /// joinability. ZWJ (U+200D), tatweel (U+0640).
    C,
    /// Transparent. Combining mark / harakat — does not participate in
    /// joining; the chain skips over T chars when computing adjacency.
    T,
}

/// The contextual form chosen for a given character within a run. Maps
/// 1:1 onto the four standard Arabic OpenType feature tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoiningForm {
    /// Standalone — neither side joins. Apply the `isol` feature.
    Isol,
    /// Initial — joins on the right only (left edge of a chain). Apply
    /// the `init` feature.
    Init,
    /// Medial — both sides join (interior of a chain). Apply the `medi`
    /// feature.
    Medi,
    /// Final — joins on the left only (right edge of a chain). Apply
    /// the `fina` feature.
    Fina,
}

impl JoiningForm {
    /// The OpenType feature tag (4-byte little-endian-as-bytes ASCII)
    /// that selects this form's substitution.
    pub fn feature_tag(self) -> [u8; 4] {
        match self {
            Self::Isol => *b"isol",
            Self::Init => *b"init",
            Self::Medi => *b"medi",
            Self::Fina => *b"fina",
        }
    }
}

/// Coarse script classification used to decide which feature tag list
/// the shaper applies to a run. Only the scripts that need contextual
/// shaping are enumerated; everything else collapses to `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    /// Arabic block + supplements + presentation forms (U+0600..U+06FF,
    /// U+0750..U+077F, U+08A0..U+08FF, U+FB50..U+FDFF, U+FE70..U+FEFF).
    Arabic,
    /// Hebrew block + Alphabetic Presentation Forms-A Hebrew range
    /// (U+0590..U+05FF, U+FB1D..U+FB4F).
    Hebrew,
    /// Devanagari block (U+0900..U+097F). Hindi / Marathi / Sanskrit /
    /// Nepali. Round 8 added cluster-based shaping — see
    /// [`super::indic`] for the cluster machine and
    /// [`super::indic::devanagari_feature_tags`] for the
    /// substitution-feature application order.
    Devanagari,
    /// Bengali block (U+0980..U+09FF). Bengali / Assamese / Manipuri.
    /// Round 10 added cluster-based shaping — same broad shape as
    /// Devanagari (halant-driven conjuncts, reph rule for RA U+09B0,
    /// pre-base matra reorder) but Bengali has THREE pre-base matras
    /// (U+09BF / U+09C7 / U+09C8) instead of Devanagari's one.
    Bengali,
    /// Tamil block (U+0B80..U+0BFF). Tamil. Round 10 added
    /// minimal cluster-based shaping: pre-base matra reorder (U+0BC6 /
    /// U+0BC7 / U+0BC8) only — no reph (Tamil RA renders in-line),
    /// no nukta, no conjunct formation in the modern orthography.
    Tamil,
    /// Gurmukhi block (U+0A00..U+0A7F). Punjabi. Round 11 added
    /// halant-driven cluster machine: pre-base matra reorder
    /// (U+0A3F sign "i"); reph rare in modern usage (RA U+0A30 sets
    /// the flag for fonts that ship a `rphf` lookup, callers without
    /// one fall back to in-line RA rendering).
    Gurmukhi,
    /// Gujarati block (U+0A80..U+0AFF). Gujarati. Round 11 added —
    /// closest in shape to Devanagari (halant-driven conjuncts;
    /// pre-base matra U+0ABF; reph rule on RA U+0AB0).
    Gujarati,
    /// Telugu block (U+0C00..U+0C7F). Telugu. Round 11 added —
    /// reph identification on RA U+0C30 plus pre-base matra reorder
    /// for U+0C46 / U+0C47 / U+0C48 (e / ee / ai). The Telugu split
    /// vowels (U+0C46 + U+0C56) decompose to a pre-base + post-base
    /// pair under NFD; the cluster machine flags the pre-base
    /// component for reorder.
    Telugu,
    /// Kannada block (U+0C80..U+0CFF). Kannada. Round 11 added —
    /// similar shape to Telugu (reph on RA U+0CB0; pre-base matras
    /// U+0CC6 / U+0CC7 / U+0CC8) with its own codepoints + halant
    /// (U+0CCD).
    Kannada,
    /// Malayalam block (U+0D00..U+0D7F). Malayalam. Round 11 added —
    /// pre-base matras U+0D46 / U+0D47 / U+0D48 plus the chillu
    /// (half-form) characters U+0D7A..U+0D7F treated as
    /// consonants (they are NFC-stable independent codepoints in modern
    /// Malayalam orthography). No reph in modern Malayalam — chillu
    /// replaces the historic reph rendering.
    Malayalam,
    /// Oriya block (U+0B00..U+0B7F). Oriya / Odia. Round 11 added —
    /// reph identification on RA U+0B30 plus pre-base matra reorder
    /// for U+0B47 / U+0B48 / U+0B4B / U+0B4C (Oriya is unusual in that
    /// the precomposed o / au matras are themselves pre-base after
    /// canonical decomposition). Halant U+0B4D drives conjuncts.
    Oriya,
    /// Anything else — Latin, CJK, Cyrillic, Greek, etc.
    Other,
}

/// Detect the script of `ch`. Returns [`Script::Other`] for any
/// codepoint not in one of the explicitly-handled blocks.
pub fn script_of(ch: char) -> Script {
    let cp = ch as u32;
    if (0x0600..=0x06FF).contains(&cp)
        || (0x0750..=0x077F).contains(&cp)
        || (0x08A0..=0x08FF).contains(&cp)
        || (0xFB50..=0xFDFF).contains(&cp)
        || (0xFE70..=0xFEFF).contains(&cp)
    {
        return Script::Arabic;
    }
    if (0x0590..=0x05FF).contains(&cp) || (0xFB1D..=0xFB4F).contains(&cp) {
        return Script::Hebrew;
    }
    if (0x0900..=0x097F).contains(&cp) {
        return Script::Devanagari;
    }
    if (0x0980..=0x09FF).contains(&cp) {
        return Script::Bengali;
    }
    if (0x0B80..=0x0BFF).contains(&cp) {
        return Script::Tamil;
    }
    if (0x0A00..=0x0A7F).contains(&cp) {
        return Script::Gurmukhi;
    }
    if (0x0A80..=0x0AFF).contains(&cp) {
        return Script::Gujarati;
    }
    if (0x0C00..=0x0C7F).contains(&cp) {
        return Script::Telugu;
    }
    if (0x0C80..=0x0CFF).contains(&cp) {
        return Script::Kannada;
    }
    if (0x0D00..=0x0D7F).contains(&cp) {
        return Script::Malayalam;
    }
    if (0x0B00..=0x0B7F).contains(&cp) {
        return Script::Oriya;
    }
    Script::Other
}

/// Feature tags the shaper should attempt to apply for a run of the
/// given script. Arabic returns the four joining features; Devanagari
/// returns the spec-mandated Indic substitution + presentation feature
/// chain (round 8); Hebrew exposes `ccmp` so future mark-composition
/// lookups can hook in. The shape pipeline ignores tags it doesn't
/// know how to apply.
pub fn feature_tags_for_run(script: Script) -> Vec<[u8; 4]> {
    match script {
        Script::Arabic => vec![*b"isol", *b"init", *b"medi", *b"fina"],
        Script::Hebrew => vec![*b"ccmp"],
        Script::Devanagari => super::indic::devanagari_feature_tags(),
        Script::Bengali => super::indic::bengali_feature_tags(),
        Script::Tamil => super::indic::tamil_feature_tags(),
        Script::Gurmukhi => super::indic::gurmukhi_feature_tags(),
        Script::Gujarati => super::indic::gujarati_feature_tags(),
        Script::Telugu => super::indic::telugu_feature_tags(),
        Script::Kannada => super::indic::kannada_feature_tags(),
        Script::Malayalam => super::indic::malayalam_feature_tags(),
        Script::Oriya => super::indic::oriya_feature_tags(),
        Script::Other => Vec::new(),
    }
}

/// Look up the Unicode joining class for `ch`. The table covers the
/// Arabic + Syriac + Arabic Supplement + Arabic Extended-A blocks plus
/// the general-category-Mn combining marks that overlap them.
///
/// Codepoints outside the joining-aware blocks return [`JoiningClass::U`]
/// — the safe "non-joining" default that breaks any chain on both
/// sides. This matches the UCD's "no entry → U" rule.
pub fn joining_class(ch: char) -> JoiningClass {
    let cp = ch as u32;
    // Fast-path: outside any joining-aware block → U.
    let in_arabic_block = (0x0600..=0x06FF).contains(&cp)
        || (0x0750..=0x077F).contains(&cp)
        || (0x0870..=0x089F).contains(&cp)
        || (0x08A0..=0x08FF).contains(&cp)
        || (0xFB50..=0xFDFF).contains(&cp)
        || (0xFE70..=0xFEFF).contains(&cp);
    let in_syriac_block = (0x0700..=0x074F).contains(&cp);
    let in_zwj_zwnj = cp == 0x200C || cp == 0x200D;
    if !in_arabic_block && !in_syriac_block && !in_zwj_zwnj {
        return JoiningClass::U;
    }

    // Joining-causing: ZWJ, tatweel, kashida-like.
    if cp == 0x200D || cp == 0x0640 || cp == 0x07FA {
        return JoiningClass::C;
    }
    // Non-joiner: ZWNJ explicitly *blocks* joining.
    if cp == 0x200C {
        return JoiningClass::U;
    }

    // Transparent: combining marks (general category Mn) within the
    // Arabic / Syriac blocks. Round-7 enumerates the dense ranges
    // explicitly rather than carrying the full UCD; the omitted
    // codepoints fall through to U which is also acceptable for marks
    // in this round (a marked char that's mistakenly U-classified
    // becomes a chain break, which is visually conservative).
    if is_transparent_mark(cp) {
        return JoiningClass::T;
    }

    // Hand-curated joining-class table for the Arabic letters we
    // actually shape. Sourced from `ArabicShaping.txt` (UCD).
    match cp {
        // -- Arabic letters in U+0620..U+064A ----------------------
        // Hamza variants (R = right-joining for hamza-on-base where
        // applicable; bare hamza U+0621 is U).
        0x0621 => JoiningClass::U, // ARABIC LETTER HAMZA
        0x0622 => JoiningClass::R, // ALEF WITH MADDA ABOVE
        0x0623 => JoiningClass::R, // ALEF WITH HAMZA ABOVE
        0x0624 => JoiningClass::R, // WAW WITH HAMZA ABOVE
        0x0625 => JoiningClass::R, // ALEF WITH HAMZA BELOW
        0x0626 => JoiningClass::D, // YEH WITH HAMZA ABOVE
        0x0627 => JoiningClass::R, // ALEF
        0x0628 => JoiningClass::D, // BEH
        0x0629 => JoiningClass::R, // TEH MARBUTA
        0x062A => JoiningClass::D, // TEH
        0x062B => JoiningClass::D, // THEH
        0x062C => JoiningClass::D, // JEEM
        0x062D => JoiningClass::D, // HAH
        0x062E => JoiningClass::D, // KHAH
        0x062F => JoiningClass::R, // DAL
        0x0630 => JoiningClass::R, // THAL
        0x0631 => JoiningClass::R, // REH
        0x0632 => JoiningClass::R, // ZAIN
        0x0633 => JoiningClass::D, // SEEN
        0x0634 => JoiningClass::D, // SHEEN
        0x0635 => JoiningClass::D, // SAD
        0x0636 => JoiningClass::D, // DAD
        0x0637 => JoiningClass::D, // TAH
        0x0638 => JoiningClass::D, // ZAH
        0x0639 => JoiningClass::D, // AIN
        0x063A => JoiningClass::D, // GHAIN
        // 0x063B..0x063F are extra letter forms — mostly D.
        0x063B..=0x063F => JoiningClass::D,
        // 0x0640 already handled above (tatweel = C).
        0x0641 => JoiningClass::D, // FEH
        0x0642 => JoiningClass::D, // QAF
        0x0643 => JoiningClass::D, // KAF
        0x0644 => JoiningClass::D, // LAM
        0x0645 => JoiningClass::D, // MEEM
        0x0646 => JoiningClass::D, // NOON
        0x0647 => JoiningClass::D, // HEH
        0x0648 => JoiningClass::R, // WAW
        0x0649 => JoiningClass::D, // ALEF MAKSURA (D in modern usage)
        0x064A => JoiningClass::D, // YEH
        // 0x064B..0x065F harakat (already classified T above).
        // -- Extended Arabic letters U+066E..U+06D3 ---------------
        0x066E..=0x066F => JoiningClass::D,
        0x0671..=0x0673 => JoiningClass::R,
        0x0674 => JoiningClass::U,
        0x0675..=0x0677 => JoiningClass::R,
        0x0678..=0x0687 => JoiningClass::D,
        0x0688..=0x0699 => JoiningClass::R,
        0x069A..=0x06A9 => JoiningClass::D,
        0x06AA => JoiningClass::R,
        0x06AB..=0x06BF => JoiningClass::D,
        0x06C0 => JoiningClass::R,
        0x06C1..=0x06C2 => JoiningClass::D,
        0x06C3..=0x06CB => JoiningClass::R,
        0x06CC => JoiningClass::D,
        0x06CD => JoiningClass::R,
        0x06CE => JoiningClass::D,
        0x06CF => JoiningClass::R,
        0x06D0..=0x06D1 => JoiningClass::D,
        0x06D2..=0x06D3 => JoiningClass::R,
        0x06D5 => JoiningClass::R,
        // -- Arabic Supplement (U+0750..U+077F) — all D ----------
        0x0750..=0x077F => JoiningClass::D,
        // -- Arabic Extended-A (U+08A0..U+08B4 etc.) — mostly D --
        0x08A0..=0x08B4 => JoiningClass::D,
        0x08B6..=0x08BD => JoiningClass::D,
        // Presentation forms — typically isolated by construction;
        // returning U keeps them out of joining chains.
        0xFB50..=0xFDFF => JoiningClass::U,
        0xFE70..=0xFEFF => JoiningClass::U,
        // Anything else in the joining-aware blocks → U (safe default).
        _ => JoiningClass::U,
    }
}

/// True when `cp` is a transparent (combining) mark within the
/// joining-aware blocks. Covers Arabic harakat, shadda, sukun, dagger
/// alef, and the Mn marks in the Syriac and Arabic Supplement blocks.
fn is_transparent_mark(cp: u32) -> bool {
    // Arabic harakat + tanwin + shadda + sukun + maddah etc.
    if (0x0610..=0x061A).contains(&cp) {
        return true;
    }
    if (0x064B..=0x065F).contains(&cp) {
        return true;
    }
    if cp == 0x0670 {
        return true;
    } // ARABIC LETTER SUPERSCRIPT ALEF
    if (0x06D6..=0x06DC).contains(&cp) {
        return true;
    }
    if (0x06DF..=0x06E4).contains(&cp) {
        return true;
    }
    if (0x06E7..=0x06E8).contains(&cp) {
        return true;
    }
    if (0x06EA..=0x06ED).contains(&cp) {
        return true;
    }
    if (0x08D3..=0x08E1).contains(&cp) {
        return true;
    }
    if (0x08E3..=0x08FF).contains(&cp) {
        return true;
    }
    // Syriac marks.
    if (0x0711..=0x0711).contains(&cp) {
        return true;
    }
    if (0x0730..=0x074A).contains(&cp) {
        return true;
    }
    false
}

/// Compute the chosen [`JoiningForm`] for every character in `chars`,
/// applying the joining-adjacency state machine described in the module
/// docs.
///
/// Inputs are assumed to be in **logical order** (post-bidi). The
/// returned `Vec` has the same length as `chars`. T-class chars receive
/// the same form as the most recent non-T base character so the caller
/// can blindly index by char position.
pub fn compute_forms(chars: &[char]) -> Vec<JoiningForm> {
    let n = chars.len();
    let mut forms = vec![JoiningForm::Isol; n];
    if n == 0 {
        return forms;
    }
    let classes: Vec<JoiningClass> = chars.iter().map(|&c| joining_class(c)).collect();

    // Helper: index of the previous non-T char, or None.
    let prev_non_t = |i: usize| -> Option<usize> {
        let mut j = i;
        while j > 0 {
            j -= 1;
            if classes[j] != JoiningClass::T {
                return Some(j);
            }
        }
        None
    };
    // Helper: index of the next non-T char, or None.
    let next_non_t = |i: usize| -> Option<usize> {
        let mut j = i + 1;
        while j < n {
            if classes[j] != JoiningClass::T {
                return Some(j);
            }
            j += 1;
        }
        None
    };

    for i in 0..n {
        let cls = classes[i];
        if cls == JoiningClass::T {
            // Resolved later — inherit from the preceding non-T base.
            continue;
        }
        // "left_joins" = the previous non-T char can extend its joining
        // chain to this char. A previous {D, L, C} can do so. Note
        // that a previous U or R cannot — R only joins to its left
        // neighbour (i.e. the *char before it*), not to its right.
        let left_can_join = matches!(
            prev_non_t(i).map(|j| classes[j]),
            Some(JoiningClass::D) | Some(JoiningClass::L) | Some(JoiningClass::C)
        );
        // "right_joins" = the next non-T char can extend its chain back
        // to this one. Next {D, R, C} can do so.
        let right_can_join = matches!(
            next_non_t(i).map(|j| classes[j]),
            Some(JoiningClass::D) | Some(JoiningClass::R) | Some(JoiningClass::C)
        );
        // Now intersect with what *this* char allows on each side:
        //   - U: never joins → always Isol.
        //   - R: joins on the left only.
        //   - L: joins on the right only.
        //   - D: joins on both sides.
        //   - C: joins on both sides (joining-causing acts as D for
        //     the purpose of form selection).
        let (this_left, this_right) = match cls {
            JoiningClass::U => (false, false),
            JoiningClass::R => (true, false),
            JoiningClass::L => (false, true),
            JoiningClass::D | JoiningClass::C => (true, true),
            JoiningClass::T => unreachable!(),
        };
        let joins_left = left_can_join && this_left;
        let joins_right = right_can_join && this_right;
        forms[i] = match (joins_left, joins_right) {
            (false, false) => JoiningForm::Isol,
            (false, true) => JoiningForm::Init,
            (true, true) => JoiningForm::Medi,
            (true, false) => JoiningForm::Fina,
        };
    }

    // Second pass: T chars inherit the form of the previous non-T base.
    let mut last_form = JoiningForm::Isol;
    for i in 0..n {
        if classes[i] == JoiningClass::T {
            forms[i] = last_form;
        } else {
            last_form = forms[i];
        }
    }

    forms
}

#[cfg(test)]
#[allow(non_snake_case)] // Tests reference Unicode codepoints / UCD class
                         // letters (R / D / U+062x) by their canonical
                         // capitalisation; renaming hurts readability.
mod tests {
    use super::*;

    #[test]
    fn joining_class_lookup_returns_R_for_alif_U_062() {
        // U+0627 ARABIC LETTER ALEF — the canonical right-joining
        // letter.
        assert_eq!(joining_class('\u{0627}'), JoiningClass::R);
    }

    #[test]
    fn joining_class_lookup_returns_D_for_ba_U_0628() {
        // U+0628 ARABIC LETTER BEH — dual-joining.
        assert_eq!(joining_class('\u{0628}'), JoiningClass::D);
    }

    #[test]
    fn dual_joining_letter_between_two_dual_joiners_picks_medi() {
        // BEH BEH BEH — interior BEH must be Medi.
        let chars = ['\u{0628}', '\u{0628}', '\u{0628}'];
        let forms = compute_forms(&chars);
        assert_eq!(forms[0], JoiningForm::Init);
        assert_eq!(forms[1], JoiningForm::Medi);
        assert_eq!(forms[2], JoiningForm::Fina);
    }

    #[test]
    fn dual_joining_letter_at_start_picks_init() {
        // BEH at start of a 2-char chain BEH+TEH.
        let chars = ['\u{0628}', '\u{062A}'];
        let forms = compute_forms(&chars);
        assert_eq!(forms[0], JoiningForm::Init);
        assert_eq!(forms[1], JoiningForm::Fina);
    }

    #[test]
    fn right_joining_letter_at_end_picks_fina() {
        // BEH then ALEF — BEH (D) is Init, ALEF (R) joins-left so
        // it becomes Fina.
        let chars = ['\u{0628}', '\u{0627}'];
        let forms = compute_forms(&chars);
        assert_eq!(forms[0], JoiningForm::Init);
        assert_eq!(forms[1], JoiningForm::Fina);
    }

    #[test]
    fn transparent_combining_mark_does_not_break_chain() {
        // BEH FATHA BEH — the FATHA (U+064E, T) sits between two
        // dual-joiners; the chain must skip it, so the second BEH
        // remains a continuation (Fina here, since the chain ends).
        // The mark inherits Init from the preceding BEH.
        let chars = ['\u{0628}', '\u{064E}', '\u{0628}'];
        let forms = compute_forms(&chars);
        assert_eq!(forms[0], JoiningForm::Init);
        assert_eq!(forms[1], JoiningForm::Init); // mark inherits
        assert_eq!(forms[2], JoiningForm::Fina);
    }

    #[test]
    fn alef_after_lam_in_la_word_picks_fina() {
        // LAM + ALEF — the canonical "la" sequence. LAM (D) is Init,
        // ALEF (R) joins-left → Fina.
        let chars = ['\u{0644}', '\u{0627}'];
        let forms = compute_forms(&chars);
        assert_eq!(forms[0], JoiningForm::Init);
        assert_eq!(forms[1], JoiningForm::Fina);
    }

    #[test]
    fn isolated_letter_with_no_neighbours_picks_isol() {
        let forms = compute_forms(&['\u{0628}']);
        assert_eq!(forms[0], JoiningForm::Isol);
    }

    #[test]
    fn right_joiner_followed_by_dual_joiner_breaks_chain() {
        // ALEF (R) cannot join its right neighbour, so the next BEH
        // sees no left-joiner and starts a new chain.
        // ALEF + BEH + BEH:
        //   ALEF(R) — Isol (no left, can't extend right)
        //   BEH(D)  — Init
        //   BEH(D)  — Fina
        let forms = compute_forms(&['\u{0627}', '\u{0628}', '\u{0628}']);
        assert_eq!(forms[0], JoiningForm::Isol);
        assert_eq!(forms[1], JoiningForm::Init);
        assert_eq!(forms[2], JoiningForm::Fina);
    }

    #[test]
    fn space_between_letters_breaks_chain() {
        // BEH SPACE BEH — space is U, breaks chain → both Isol.
        let chars = ['\u{0628}', ' ', '\u{0628}'];
        let forms = compute_forms(&chars);
        assert_eq!(forms[0], JoiningForm::Isol);
        assert_eq!(forms[1], JoiningForm::Isol);
        assert_eq!(forms[2], JoiningForm::Isol);
    }

    #[test]
    fn zwj_extends_chain_across_non_joiner() {
        // BEH + ZWJ + ZWJ + BEH should all participate via the C
        // class. The two ZWJs are joining-causing → Medi each, and
        // the BEHs become Init / Fina.
        let chars = ['\u{0628}', '\u{200D}', '\u{200D}', '\u{0628}'];
        let forms = compute_forms(&chars);
        assert_eq!(forms[0], JoiningForm::Init);
        assert_eq!(forms[1], JoiningForm::Medi);
        assert_eq!(forms[2], JoiningForm::Medi);
        assert_eq!(forms[3], JoiningForm::Fina);
    }

    #[test]
    fn zwnj_breaks_chain() {
        // BEH + ZWNJ + BEH — ZWNJ (U) explicitly breaks the chain.
        let chars = ['\u{0628}', '\u{200C}', '\u{0628}'];
        let forms = compute_forms(&chars);
        assert_eq!(forms[0], JoiningForm::Isol);
        assert_eq!(forms[1], JoiningForm::Isol);
        assert_eq!(forms[2], JoiningForm::Isol);
    }

    #[test]
    fn script_of_arabic_alef_is_arabic() {
        assert_eq!(script_of('\u{0627}'), Script::Arabic);
    }

    #[test]
    fn script_of_hebrew_alef_is_hebrew() {
        assert_eq!(script_of('\u{05D0}'), Script::Hebrew);
    }

    #[test]
    fn script_of_latin_a_is_other() {
        assert_eq!(script_of('A'), Script::Other);
    }

    #[test]
    fn feature_tags_for_arabic_includes_four_joining_features() {
        let tags = feature_tags_for_run(Script::Arabic);
        assert!(tags.contains(b"isol"));
        assert!(tags.contains(b"init"));
        assert!(tags.contains(b"medi"));
        assert!(tags.contains(b"fina"));
    }

    #[test]
    fn feature_tags_for_other_is_empty() {
        assert!(feature_tags_for_run(Script::Other).is_empty());
    }

    #[test]
    fn feature_tag_round_trips_per_form() {
        assert_eq!(JoiningForm::Isol.feature_tag(), *b"isol");
        assert_eq!(JoiningForm::Init.feature_tag(), *b"init");
        assert_eq!(JoiningForm::Medi.feature_tag(), *b"medi");
        assert_eq!(JoiningForm::Fina.feature_tag(), *b"fina");
    }

    #[test]
    fn empty_run_returns_empty() {
        assert!(compute_forms(&[]).is_empty());
    }

    #[test]
    fn arabic_word_alsalam_picks_expected_forms() {
        // "السلام" = ALEF LAM SEEN LAM ALEF MEEM
        // Joining classes: R D D D R D
        // Expected forms (logical order):
        //   ALEF(R)  Isol — no left, R can't join right
        //   LAM(D)   Init — right joins (SEEN D), left ALEF can't extend right
        //   SEEN(D)  Medi — both LAMs are D
        //   LAM(D)   Medi — between SEEN and ALEF (R can join left)
        //   ALEF(R)  Fina — left LAM extends, ALEF can't extend right
        //   MEEM(D)  Isol — no left (ALEF R can't extend right), no right
        let chars: Vec<char> = "السلام".chars().collect();
        let forms = compute_forms(&chars);
        assert_eq!(forms.len(), 6);
        assert_eq!(forms[0], JoiningForm::Isol);
        assert_eq!(forms[1], JoiningForm::Init);
        assert_eq!(forms[2], JoiningForm::Medi);
        assert_eq!(forms[3], JoiningForm::Medi);
        assert_eq!(forms[4], JoiningForm::Fina);
        assert_eq!(forms[5], JoiningForm::Isol);
    }
}
