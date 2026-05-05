//! Indic complex-script shaping (round 8 / round 10).
//!
//! Devanagari (Hindi / Marathi / Sanskrit / Nepali) was the first Indic
//! script we supported (round 8, commit 67a50bc). Round 10 generalises
//! the same cluster-machine pattern to two more scripts:
//!
//! - **Bengali** (U+0980..U+09FF) — Bengali / Assamese / Manipuri.
//!   Closest to Devanagari: same halant-driven conjunct formation, same
//!   reph rule (RA U+09B0 + halant U+09CD + consonant), same family of
//!   pre-base reordering matras (U+09BF "i", U+09C7 "e", U+09C8 "ai"
//!   — Bengali is unusual in that THREE matras reorder, not just one).
//! - **Tamil** (U+0B80..U+0BFF) — minimal cluster reordering. No
//!   conjunct formation in the modern orthography (each consonant is
//!   independently spelled with its own pulli / virama U+0BCD when
//!   needed). The split-vowel matras (U+0BCA = U+0BC6 + U+0BBE,
//!   U+0BCB = U+0BC7 + U+0BBE, U+0BCC = U+0BC6 + U+0BD7) carry a
//!   pre-base component that requires reordering when present.
//!
//! Unlike Arabic — which is purely contextual joining over a
//! left-to-right glyph stream — Indic shaping is **cluster-based**:
//! input characters are grouped into orthographic syllables, then
//! re-ordered + re-shaped within each cluster according to script-
//! specific rules.
//!
//! ## Scope per round
//!
//! Round 8 covered Devanagari pre-base matra reorder + reph
//! identification (without `rphf` GSUB substitution).
//!
//! Round 10 (previous round):
//! - Added Bengali + Tamil as two more scripts under the same shape.
//! - Wired the `rphf` GSUB feature to the reph identification: when a
//!   cluster has [`ClusterFlags::has_reph`] AND the active face publishes
//!   a `rphf` lookup for the script, the leading RA glyph is rewritten
//!   to its reph form via [`oxideav_ttf::Font::gsub_apply_lookup_type_1`]
//!   and the halant is dropped. See [`crate::face_chain`] for the
//!   wiring.
//!
//! Round 11 (this round):
//! - Adds six more Indic scripts: Gurmukhi, Gujarati, Telugu, Kannada,
//!   Malayalam, Oriya — each via a per-script categorisation table +
//!   `*_RULES` constant + `*_feature_tags()` function.
//! - Adds cluster-position-aware GSUB feature wiring on top of the
//!   round-10 `rphf` pattern. For every halant-suffixed consonant in a
//!   cluster we apply the appropriate per-position lookup — `half` for
//!   non-final consonants, `pref` / `blwf` / `abvf` / `pstf` for the
//!   pre-base / below-base / above-base / post-base components of
//!   split-vowel matras and Telugu/Kannada/Malayalam-style conjuncts.
//!   The presentation-pass features `pres` / `psts` / `abvs` / `blws`
//!   are then applied to every glyph in the cluster (single
//!   substitution); coverage misses pass through unchanged.
//!
//! ## References
//!
//! - Unicode 15.1 Standard Annex #15 (Indic syllabic categories).
//! - Unicode 15.1 Standard Annex #29 (text segmentation; grapheme
//!   cluster baseline).
//! - Microsoft OpenType Layout — *Creating and supporting OpenType
//!   fonts for the Devanagari script* (the canonical description of
//!   the cluster reorder rules + GSUB feature application order).
//! - Microsoft OpenType Layout — *Creating and supporting OpenType
//!   fonts for Indic scripts* (Bengali / Tamil / Telugu / Gujarati /
//!   Gurmukhi / Kannada / Malayalam / Oriya).
//!
//! No HarfBuzz / FreeType / pango / ICU layout source consulted. The
//! algorithms are clean-room implementations derived from the Unicode +
//! OpenType specs above plus the per-script `Shaping` informative
//! examples in the OpenType layout doc.

#![allow(clippy::manual_range_contains)]

/// Indic syllabic category. Names are short for readability;
/// see the per-variant docs for the full Unicode classification.
///
/// The same enum is used across all supported Indic scripts — what
/// differs between scripts is the per-codepoint classifier (e.g.
/// [`devanagari_category`] vs [`bengali_category`]) and the cluster /
/// feature application rules that consume it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicCategory {
    /// Independent consonant (base). Drives cluster start / base
    /// selection. Per-script ranges:
    /// - Devanagari U+0915..U+0939, U+0958..U+095F, U+0978..U+097F.
    /// - Bengali U+0995..U+09B9 (with gaps), U+09DC..U+09DF,
    ///   U+09F0..U+09F1.
    /// - Tamil U+0B95..U+0BB9 (with gaps).
    Consonant,
    /// Independent vowel — acts as a base for the cluster but does not
    /// chain via halant. Per-script ranges:
    /// - Devanagari U+0904..U+0914, U+0960..U+0961.
    /// - Bengali U+0985..U+0994.
    /// - Tamil U+0B85..U+0B94.
    Vowel,
    /// Halant / virama — suppresses the inherent vowel of the
    /// preceding consonant; when followed by another consonant it
    /// forms a conjunct (both stay in the same cluster).
    /// Per-script codepoints:
    /// - Devanagari U+094D.
    /// - Bengali U+09CD.
    /// - Tamil U+0BCD (often called "pulli" in Tamil contexts).
    Halant,
    /// Pre-base reordering matra — vowel sign that appears AFTER its
    /// base consonant in logical order but renders VISUALLY BEFORE it.
    /// The reorderer in [`reorder_cluster`] swaps it to the front of
    /// the cluster. Per-script codepoints:
    /// - Devanagari U+093F (sign "i").
    /// - Bengali U+09BF (sign "i"), U+09C7 (sign "e"), U+09C8
    ///   (sign "ai") — Bengali is unusual in having THREE.
    /// - Tamil U+0BC6 (sign "e"), U+0BC7 (sign "ee"), U+0BC8 (sign "ai")
    ///   — Tamil's e/ee/ai matras are pre-base rather than post-base.
    PreBaseMatra,
    /// Vowel sign / matra (other than pre-base). Stays in its logical
    /// position within the cluster.
    Matra,
    /// Nukta — combining dot-below; binds tightly to the
    /// preceding consonant (forms a "nukta'd" consonant). Per-script:
    /// - Devanagari U+093C.
    /// - Bengali U+09BC.
    /// - Tamil: no nukta in the modern orthography (no codepoint
    ///   classified `Nukta` for Tamil).
    Nukta,
    /// Anusvara / candrabindu / visarga — bindu marks that attach to
    /// the cluster end. Per-script:
    /// - Devanagari U+0900..U+0903.
    /// - Bengali U+0981..U+0983.
    /// - Tamil U+0B82, U+0B83.
    Bindu,
    /// Avagraha + danda + double-danda + the digit block + various
    /// miscellaneous symbols. Treated as cluster-breaking
    /// (each is its own cluster).
    Symbol,
    /// Anything outside the script's main block. Treated as a cluster
    /// boundary — an Indic cluster never crosses the script boundary.
    Other,
}

/// Look up the Indic category for `ch` within the Devanagari block.
/// Codepoints outside U+0900..U+097F return [`IndicCategory::Other`].
///
/// The classification follows the Unicode `IndicSyllabicCategory.txt`
/// and `IndicPositionalCategory.txt` properties — but condensed to the
/// nine categories the cluster machine actually distinguishes.
pub fn devanagari_category(ch: char) -> IndicCategory {
    let cp = ch as u32;
    // Fast-path: outside the Devanagari block entirely.
    if cp < 0x0900 || cp > 0x097F {
        return IndicCategory::Other;
    }
    match cp {
        // Bindu marks (anusvara / candrabindu / visarga / inverted
        // candrabindu).
        0x0900..=0x0903 => IndicCategory::Bindu,
        // Independent vowels (A through AU).
        0x0904..=0x0914 => IndicCategory::Vowel,
        // Consonants KA..HA + extended consonants (NNNA, RRA, etc.).
        0x0915..=0x0939 => IndicCategory::Consonant,
        // Nukta — combining dot below.
        0x093C => IndicCategory::Nukta,
        // Avagraha (sign) — symbol; treated as cluster break.
        0x093D => IndicCategory::Symbol,
        // Vowel signs (matras) other than pre-base "i".
        0x093A | 0x093B => IndicCategory::Matra,
        // Post-base matra AA (U+093E) — the most common matra in
        // running Hindi text.
        0x093E => IndicCategory::Matra,
        // Pre-base matra "i" — the only matra that needs reordering
        // in modern Devanagari.
        0x093F => IndicCategory::PreBaseMatra,
        // The post-base, above-base, and below-base matras.
        0x0940..=0x094C => IndicCategory::Matra,
        // Halant / virama.
        0x094D => IndicCategory::Halant,
        0x094E..=0x094F => IndicCategory::Matra,
        // Stress signs / udatta + anudatta + grave + acute + Vedic
        // marks.
        0x0951..=0x0954 => IndicCategory::Bindu,
        // Vowel signs UE / UUE / SHORT_E (Marathi extensions).
        0x0955..=0x0957 => IndicCategory::Matra,
        // Additional nukta'd consonants (QA..YYA).
        0x0958..=0x095F => IndicCategory::Consonant,
        // Vocalic L + LL (independent vowels).
        0x0960..=0x0961 => IndicCategory::Vowel,
        // Vowel signs vocalic L + LL.
        0x0962..=0x0963 => IndicCategory::Matra,
        // Danda + double-danda + abbreviation sign + Devanagari ".".
        // All cluster-breaking symbols.
        0x0964..=0x096F => IndicCategory::Symbol, // includes digits 0966..096F
        0x0970..=0x0977 => IndicCategory::Symbol,
        // Extended consonants (Sindhi / Marathi etc.).
        0x0978..=0x097F => IndicCategory::Consonant,
        // Already enumerated everything in the block — exhaustive
        // match for clarity. Anything we missed defaults to Symbol
        // (cluster-breaking) which is the conservative choice.
        _ => IndicCategory::Symbol,
    }
}

/// Look up the Indic category for `ch` within the Bengali block.
/// Codepoints outside U+0980..U+09FF return [`IndicCategory::Other`].
///
/// Bengali shares Devanagari's structural shape (halant U+09CD glues
/// consonants into conjuncts; bindus attach to the cluster end) but
/// has THREE pre-base reordering matras (U+09BF "i", U+09C7 "e",
/// U+09C8 "ai") instead of Devanagari's one. The reph rule is the
/// same shape — RA U+09B0 + halant + consonant.
pub fn bengali_category(ch: char) -> IndicCategory {
    let cp = ch as u32;
    if cp < 0x0980 || cp > 0x09FF {
        return IndicCategory::Other;
    }
    match cp {
        // U+0980 BENGALI ANJI — sign; cluster-breaking.
        0x0980 => IndicCategory::Symbol,
        // Bindu marks: candrabindu / anusvara / visarga.
        0x0981..=0x0983 => IndicCategory::Bindu,
        // U+0984 unassigned.
        // Independent vowels A..AU (with gaps at U+098D, U+098E,
        // U+0991, U+0992 historically). We classify the entire span
        // as Vowel — assigned points are vowels; unassigned points
        // (which the font's cmap won't have anyway) fall through.
        0x0985..=0x098C => IndicCategory::Vowel,
        0x098F..=0x0990 => IndicCategory::Vowel,
        0x0993..=0x0994 => IndicCategory::Vowel,
        // Consonants KA..HA — Bengali block consonants run U+0995..
        // U+09B9 with gaps at the same positions Devanagari has gaps.
        0x0995..=0x09A8 => IndicCategory::Consonant,
        // U+09A9 unassigned.
        0x09AA..=0x09B0 => IndicCategory::Consonant,
        // U+09B1 unassigned.
        0x09B2 => IndicCategory::Consonant,
        // U+09B3..U+09B5 unassigned.
        0x09B6..=0x09B9 => IndicCategory::Consonant,
        // U+09BA, U+09BB unassigned.
        // Nukta — combining dot below.
        0x09BC => IndicCategory::Nukta,
        // Avagraha — symbol.
        0x09BD => IndicCategory::Symbol,
        // Vowel sign AA (post-base).
        0x09BE => IndicCategory::Matra,
        // Pre-base matra "i".
        0x09BF => IndicCategory::PreBaseMatra,
        // Vowel signs II / U / UU / R / RR (post-base + below-base).
        0x09C0..=0x09C4 => IndicCategory::Matra,
        // U+09C5, U+09C6 unassigned.
        // Pre-base matras "e" and "ai".
        0x09C7..=0x09C8 => IndicCategory::PreBaseMatra,
        // U+09C9, U+09CA unassigned (the slot for "o" / "au" — these
        // are encoded as 2-character sequences combining U+09C7 +
        // U+09BE / U+09D7 in modern Bengali).
        // Vowel signs "o" and "au" — these ARE encoded as U+09CB and
        // U+09CC (canonical decomposition: U+09C7 + U+09BE / U+09D7).
        // We treat them as post-base matras (the cluster machine sees
        // the canonical-equivalent form when text has been NFC-normalised;
        // the precomposed code points themselves are post-base).
        0x09CB..=0x09CC => IndicCategory::Matra,
        // Halant / virama (Bengali "hashanta").
        0x09CD => IndicCategory::Halant,
        // U+09CE BENGALI LETTER KHANDA TA — special consonant form.
        0x09CE => IndicCategory::Consonant,
        // U+09CF..U+09D6 unassigned.
        // U+09D7 BENGALI AU LENGTH MARK — combining mark used in the
        // canonical decomposition of U+09CC. Post-base position.
        0x09D7 => IndicCategory::Matra,
        // U+09D8..U+09DB unassigned.
        // RRA, RHA — additional consonants.
        0x09DC..=0x09DD => IndicCategory::Consonant,
        // U+09DE unassigned.
        // YYA — additional consonant.
        0x09DF => IndicCategory::Consonant,
        // Vocalic R / L (independent vowels).
        0x09E0..=0x09E1 => IndicCategory::Vowel,
        // Vowel signs vocalic L (matras).
        0x09E2..=0x09E3 => IndicCategory::Matra,
        // U+09E4, U+09E5 unassigned.
        // Digits + miscellaneous symbols.
        0x09E6..=0x09EF => IndicCategory::Symbol, // digits 0..9
        // RUPEE MARK / RUPEE SIGN / NUMERATOR / etc.
        0x09F0 => IndicCategory::Consonant, // BENGALI LETTER RA WITH MIDDLE DIAGONAL (Assamese)
        0x09F1 => IndicCategory::Consonant, // BENGALI LETTER RA WITH LOWER DIAGONAL (Assamese)
        0x09F2..=0x09FF => IndicCategory::Symbol,
        _ => IndicCategory::Symbol,
    }
}

/// Look up the Indic category for `ch` within the Tamil block.
/// Codepoints outside U+0B80..U+0BFF return [`IndicCategory::Other`].
///
/// Tamil's cluster machine is the simplest of the supported scripts:
/// - No nukta (no U+0BBC slot).
/// - No reph rule — Tamil RA (U+0BB0) does NOT form a superscript even
///   in RA+halant+consonant sequence; the halant + RA is rendered
///   in-line.
/// - The pulli / virama (U+0BCD) DOES suppress the inherent vowel
///   like other Indic halants, but Tamil orthography prefers
///   independent consonants to conjuncts in most cases.
/// - THREE pre-base matras: U+0BC6 "e", U+0BC7 "ee", U+0BC8 "ai".
/// - Two-character vowel signs U+0BCA/U+0BCB/U+0BCC are precomposed
///   forms of pre-base + post-base components; we treat the
///   precomposed codepoints as post-base matras (the canonical
///   decomposition is the responsibility of the upstream NFC
///   normaliser).
pub fn tamil_category(ch: char) -> IndicCategory {
    let cp = ch as u32;
    if cp < 0x0B80 || cp > 0x0BFF {
        return IndicCategory::Other;
    }
    match cp {
        // U+0B80, U+0B81 unassigned.
        // U+0B82 anusvara, U+0B83 visarga.
        0x0B82..=0x0B83 => IndicCategory::Bindu,
        // Independent vowels A..AU (with standard Tamil gaps at
        // U+0B8B..U+0B8D, U+0B91 — those slots are unassigned).
        0x0B85..=0x0B8A => IndicCategory::Vowel,
        0x0B8E..=0x0B90 => IndicCategory::Vowel,
        0x0B92..=0x0B94 => IndicCategory::Vowel,
        // Consonants KA..HA (with the standard Tamil gaps).
        0x0B95 => IndicCategory::Consonant,
        0x0B99..=0x0B9A => IndicCategory::Consonant,
        0x0B9C => IndicCategory::Consonant,
        0x0B9E..=0x0B9F => IndicCategory::Consonant,
        0x0BA3..=0x0BA4 => IndicCategory::Consonant,
        0x0BA8..=0x0BAA => IndicCategory::Consonant,
        0x0BAE..=0x0BB9 => IndicCategory::Consonant,
        // U+0BBA..U+0BBD unassigned (no nukta / avagraha in Tamil).
        // Vowel sign AA — post-base.
        0x0BBE => IndicCategory::Matra,
        // Vowel signs I / II / U / UU — post-base.
        0x0BBF..=0x0BC2 => IndicCategory::Matra,
        // U+0BC3..U+0BC5 unassigned.
        // Pre-base matras E / EE / AI.
        0x0BC6..=0x0BC8 => IndicCategory::PreBaseMatra,
        // U+0BC9 unassigned.
        // Two-character vowel signs O / OO / AU — precomposed forms.
        // We classify them as post-base matras: the canonical
        // decomposition (U+0BC6 + U+0BBE / U+0BD7) carries the
        // pre-base component explicitly. Callers feeding NFC-normalised
        // text get the canonical decomposition for free; raw
        // precomposed input gets a post-base matra (visually the wrong
        // position for the pre-base component, but no orthographic
        // damage — the cluster still renders).
        0x0BCA..=0x0BCC => IndicCategory::Matra,
        // Pulli / virama — Tamil's halant.
        0x0BCD => IndicCategory::Halant,
        // U+0BCE..U+0BD6 unassigned.
        // U+0BD7 AU LENGTH MARK — combining; post-base.
        0x0BD7 => IndicCategory::Matra,
        // U+0BD8..U+0BE5 unassigned.
        // Tamil digits.
        0x0BE6..=0x0BEF => IndicCategory::Symbol,
        // Tamil numbers / signs (year / month / day / etc.).
        0x0BF0..=0x0BFF => IndicCategory::Symbol,
        _ => IndicCategory::Other,
    }
}

/// Look up the Indic category for `ch` within the Gurmukhi block
/// (U+0A00..U+0A7F). Punjabi.
///
/// Gurmukhi shares Devanagari's halant-driven shape (halant U+0A4D
/// glues consonants into conjuncts) but reph is rare in modern usage —
/// fonts that ship a `rphf` lookup substitute for RA + halant; fonts
/// that don't fall back to in-line RA rendering. The cluster machine
/// flags reph regardless and lets the GSUB pass decide.
pub fn gurmukhi_category(ch: char) -> IndicCategory {
    let cp = ch as u32;
    if cp < 0x0A00 || cp > 0x0A7F {
        return IndicCategory::Other;
    }
    match cp {
        // Bindu marks (anusvara / candrabindu / visarga / udaat).
        0x0A01..=0x0A03 => IndicCategory::Bindu,
        // Independent vowels.
        0x0A05..=0x0A0A => IndicCategory::Vowel,
        0x0A0F..=0x0A10 => IndicCategory::Vowel,
        0x0A13..=0x0A14 => IndicCategory::Vowel,
        // Consonants KA..HA (with the standard Gurmukhi gaps).
        0x0A15..=0x0A28 => IndicCategory::Consonant,
        0x0A2A..=0x0A30 => IndicCategory::Consonant,
        0x0A32..=0x0A33 => IndicCategory::Consonant,
        0x0A35..=0x0A36 => IndicCategory::Consonant,
        0x0A38..=0x0A39 => IndicCategory::Consonant,
        // Nukta — combining dot below.
        0x0A3C => IndicCategory::Nukta,
        // Vowel sign AA — post-base.
        0x0A3E => IndicCategory::Matra,
        // Pre-base matra "i".
        0x0A3F => IndicCategory::PreBaseMatra,
        // Vowel signs II / U / UU — post-base / below-base.
        0x0A40..=0x0A42 => IndicCategory::Matra,
        // Vowel signs E / AI / O / AU — above-base / post-base.
        0x0A47..=0x0A48 => IndicCategory::Matra,
        0x0A4B..=0x0A4C => IndicCategory::Matra,
        // Halant / virama.
        0x0A4D => IndicCategory::Halant,
        // U+0A51 UDAAT — bindu-like.
        0x0A51 => IndicCategory::Bindu,
        // Additional consonants (KHHA, GHHA, ZA, RRA, etc.).
        0x0A59..=0x0A5C => IndicCategory::Consonant,
        0x0A5E => IndicCategory::Consonant,
        // Digits + symbols.
        0x0A66..=0x0A6F => IndicCategory::Symbol,
        0x0A70..=0x0A71 => IndicCategory::Bindu,
        // Iri / Ura / Yakash — modifier letters/consonants in some fonts.
        0x0A72..=0x0A74 => IndicCategory::Consonant,
        0x0A75 => IndicCategory::Bindu,
        _ => IndicCategory::Symbol,
    }
}

/// Look up the Indic category for `ch` within the Gujarati block
/// (U+0A80..U+0AFF). Closest in shape to Devanagari.
pub fn gujarati_category(ch: char) -> IndicCategory {
    let cp = ch as u32;
    if cp < 0x0A80 || cp > 0x0AFF {
        return IndicCategory::Other;
    }
    match cp {
        // Bindu marks.
        0x0A81..=0x0A83 => IndicCategory::Bindu,
        // Independent vowels.
        0x0A85..=0x0A8D => IndicCategory::Vowel,
        0x0A8F..=0x0A91 => IndicCategory::Vowel,
        0x0A93..=0x0A94 => IndicCategory::Vowel,
        // Consonants KA..HA.
        0x0A95..=0x0AA8 => IndicCategory::Consonant,
        0x0AAA..=0x0AB0 => IndicCategory::Consonant,
        0x0AB2..=0x0AB3 => IndicCategory::Consonant,
        0x0AB5..=0x0AB9 => IndicCategory::Consonant,
        // Nukta.
        0x0ABC => IndicCategory::Nukta,
        // Avagraha — symbol.
        0x0ABD => IndicCategory::Symbol,
        // Vowel sign AA — post-base.
        0x0ABE => IndicCategory::Matra,
        // Pre-base matra "i".
        0x0ABF => IndicCategory::PreBaseMatra,
        // Other matras (II / U / UU / R / RR / E / AI / O / AU).
        0x0AC0..=0x0AC5 => IndicCategory::Matra,
        0x0AC7..=0x0AC9 => IndicCategory::Matra,
        0x0ACB..=0x0ACC => IndicCategory::Matra,
        // Halant / virama.
        0x0ACD => IndicCategory::Halant,
        // OM symbol.
        0x0AD0 => IndicCategory::Consonant,
        // Vocalic R / L (independent vowels).
        0x0AE0..=0x0AE1 => IndicCategory::Vowel,
        // Vowel signs vocalic L (matras).
        0x0AE2..=0x0AE3 => IndicCategory::Matra,
        // Digits + Gujarati symbols.
        0x0AE6..=0x0AEF => IndicCategory::Symbol,
        0x0AF0..=0x0AFF => IndicCategory::Symbol,
        _ => IndicCategory::Symbol,
    }
}

/// Look up the Indic category for `ch` within the Telugu block
/// (U+0C00..U+0C7F).
///
/// Telugu split vowels (e.g. U+0C46 + U+0C56) decompose a precomposed
/// matra into a pre-base + post-base pair under NFD. We classify the
/// pre-base components U+0C46 / U+0C47 / U+0C48 as `PreBaseMatra` so the
/// reorderer moves them to the front of the cluster.
pub fn telugu_category(ch: char) -> IndicCategory {
    let cp = ch as u32;
    if cp < 0x0C00 || cp > 0x0C7F {
        return IndicCategory::Other;
    }
    match cp {
        // Bindu marks (combining candrabindu / anusvara / visarga).
        0x0C00..=0x0C04 => IndicCategory::Bindu,
        // Independent vowels.
        0x0C05..=0x0C0C => IndicCategory::Vowel,
        0x0C0E..=0x0C10 => IndicCategory::Vowel,
        0x0C12..=0x0C14 => IndicCategory::Vowel,
        // Consonants KA..HA.
        0x0C15..=0x0C28 => IndicCategory::Consonant,
        0x0C2A..=0x0C39 => IndicCategory::Consonant,
        // U+0C3C TELUGU SIGN NUKTA (added in Unicode 16; included for
        // forward compatibility — fonts without it cmap-miss safely).
        0x0C3C => IndicCategory::Nukta,
        // Avagraha — symbol.
        0x0C3D => IndicCategory::Symbol,
        // Post-base matras AA / I / II / U / UU / R / RR.
        0x0C3E..=0x0C44 => IndicCategory::Matra,
        // Pre-base matras E / EE / AI.
        0x0C46..=0x0C48 => IndicCategory::PreBaseMatra,
        // Post-base matras O / OO / AU (precomposed).
        0x0C4A..=0x0C4C => IndicCategory::Matra,
        // Halant / virama.
        0x0C4D => IndicCategory::Halant,
        // U+0C55 / U+0C56 — length marks (post-base / above).
        0x0C55..=0x0C56 => IndicCategory::Matra,
        // Vocalic L / LL (independent vowels).
        0x0C58..=0x0C5A => IndicCategory::Consonant,
        // Vocalic R / L matras.
        0x0C60..=0x0C61 => IndicCategory::Vowel,
        0x0C62..=0x0C63 => IndicCategory::Matra,
        // Digits + Telugu fractions/symbols.
        0x0C66..=0x0C6F => IndicCategory::Symbol,
        0x0C77..=0x0C7F => IndicCategory::Symbol,
        _ => IndicCategory::Symbol,
    }
}

/// Look up the Indic category for `ch` within the Kannada block
/// (U+0C80..U+0CFF). Similar shape to Telugu but distinct codepoints +
/// own halant U+0CCD.
pub fn kannada_category(ch: char) -> IndicCategory {
    let cp = ch as u32;
    if cp < 0x0C80 || cp > 0x0CFF {
        return IndicCategory::Other;
    }
    match cp {
        // Bindu marks.
        0x0C80..=0x0C83 => IndicCategory::Bindu,
        // Independent vowels.
        0x0C85..=0x0C8C => IndicCategory::Vowel,
        0x0C8E..=0x0C90 => IndicCategory::Vowel,
        0x0C92..=0x0C94 => IndicCategory::Vowel,
        // Consonants KA..HA.
        0x0C95..=0x0CA8 => IndicCategory::Consonant,
        0x0CAA..=0x0CB3 => IndicCategory::Consonant,
        0x0CB5..=0x0CB9 => IndicCategory::Consonant,
        // Nukta.
        0x0CBC => IndicCategory::Nukta,
        // Avagraha — symbol.
        0x0CBD => IndicCategory::Symbol,
        // Vowel sign AA — post-base.
        0x0CBE => IndicCategory::Matra,
        // Pre-base matra "i".
        0x0CBF => IndicCategory::PreBaseMatra,
        // Vowel signs II / U / UU / R / RR — post-base/right.
        0x0CC0..=0x0CC4 => IndicCategory::Matra,
        // Pre-base matras E / EE / AI.
        0x0CC6..=0x0CC8 => IndicCategory::PreBaseMatra,
        // Vowel signs O / OO / AU — post-base.
        0x0CCA..=0x0CCC => IndicCategory::Matra,
        // Halant / virama.
        0x0CCD => IndicCategory::Halant,
        // Length marks (post-base).
        0x0CD5..=0x0CD6 => IndicCategory::Matra,
        // U+0CDD / U+0CDE — additional consonants.
        0x0CDD..=0x0CDE => IndicCategory::Consonant,
        // Vocalic R / L (independent vowels).
        0x0CE0..=0x0CE1 => IndicCategory::Vowel,
        // Vowel signs vocalic L (matras).
        0x0CE2..=0x0CE3 => IndicCategory::Matra,
        // Digits + Kannada signs.
        0x0CE6..=0x0CEF => IndicCategory::Symbol,
        0x0CF1..=0x0CFF => IndicCategory::Symbol,
        _ => IndicCategory::Symbol,
    }
}

/// Look up the Indic category for `ch` within the Malayalam block
/// (U+0D00..U+0D7F).
///
/// Malayalam orthography uses chillu (half-form) characters
/// U+0D7A..U+0D7F as NFC-stable independent codepoints — they replace
/// the historic reph rendering. We classify them as `Consonant` so the
/// cluster machine treats them as bases that can start a new cluster.
pub fn malayalam_category(ch: char) -> IndicCategory {
    let cp = ch as u32;
    if cp < 0x0D00 || cp > 0x0D7F {
        return IndicCategory::Other;
    }
    match cp {
        // Bindu marks.
        0x0D00..=0x0D03 => IndicCategory::Bindu,
        // Independent vowels.
        0x0D05..=0x0D0C => IndicCategory::Vowel,
        0x0D0E..=0x0D10 => IndicCategory::Vowel,
        0x0D12..=0x0D14 => IndicCategory::Vowel,
        // Consonants KA..HA + extended (LLLA, ZHA, etc.).
        0x0D15..=0x0D3A => IndicCategory::Consonant,
        // Nukta — combining dot below (Unicode 14+).
        0x0D3B..=0x0D3C => IndicCategory::Nukta,
        // Avagraha — symbol.
        0x0D3D => IndicCategory::Symbol,
        // Vowel sign AA — post-base.
        0x0D3E => IndicCategory::Matra,
        // Vowel signs I / II / U / UU / R / RR — post-base.
        0x0D3F..=0x0D44 => IndicCategory::Matra,
        // Pre-base matras E / EE / AI.
        0x0D46..=0x0D48 => IndicCategory::PreBaseMatra,
        // Vowel signs O / OO / AU — pre+post canonical decomposition;
        // we classify them as post-base (visually the cluster machine
        // sees the canonical form when callers feed NFC-normalised
        // text).
        0x0D4A..=0x0D4C => IndicCategory::Matra,
        // Halant / virama.
        0x0D4D => IndicCategory::Halant,
        // U+0D4E DOT REPH — pre-base mark (used in Malayalam to
        // indicate a reph-like form). Classified as a matra so it stays
        // attached to its cluster but doesn't trigger reorder.
        0x0D4E => IndicCategory::Matra,
        // Length mark (post-base).
        0x0D57 => IndicCategory::Matra,
        // Fraction / numerator signs (treated as cluster-breaking).
        0x0D58..=0x0D5F => IndicCategory::Symbol,
        // Vocalic R / L (independent vowels).
        0x0D60..=0x0D61 => IndicCategory::Vowel,
        // Vowel signs vocalic L (matras).
        0x0D62..=0x0D63 => IndicCategory::Matra,
        // Digits + Malayalam fractions.
        0x0D66..=0x0D6F => IndicCategory::Symbol,
        0x0D70..=0x0D79 => IndicCategory::Symbol,
        // Chillu characters (independent half-forms) — consonants in
        // their own right.
        0x0D7A..=0x0D7F => IndicCategory::Consonant,
        _ => IndicCategory::Symbol,
    }
}

/// Look up the Indic category for `ch` within the Oriya / Odia block
/// (U+0B00..U+0B7F).
///
/// Oriya is unusual in that the precomposed o / au matras
/// (U+0B4B / U+0B4C) themselves carry pre-base components after
/// canonical decomposition. The cluster machine flags U+0B47 / U+0B48 /
/// U+0B4B / U+0B4C as pre-base matras to keep the visual ordering
/// correct without depending on the upstream NFC normaliser.
pub fn oriya_category(ch: char) -> IndicCategory {
    let cp = ch as u32;
    if cp < 0x0B00 || cp > 0x0B7F {
        return IndicCategory::Other;
    }
    match cp {
        // Bindu marks (candrabindu / anusvara / visarga).
        0x0B01..=0x0B03 => IndicCategory::Bindu,
        // Independent vowels.
        0x0B05..=0x0B0C => IndicCategory::Vowel,
        0x0B0F..=0x0B10 => IndicCategory::Vowel,
        0x0B13..=0x0B14 => IndicCategory::Vowel,
        // Consonants KA..HA.
        0x0B15..=0x0B28 => IndicCategory::Consonant,
        0x0B2A..=0x0B30 => IndicCategory::Consonant,
        0x0B32..=0x0B33 => IndicCategory::Consonant,
        0x0B35..=0x0B39 => IndicCategory::Consonant,
        // Nukta.
        0x0B3C => IndicCategory::Nukta,
        // Avagraha — symbol.
        0x0B3D => IndicCategory::Symbol,
        // Vowel sign AA — post-base.
        0x0B3E => IndicCategory::Matra,
        // Above-base matra "i" (Oriya I sign sits ABOVE the base —
        // not a pre-base reorder). Treat as a regular matra.
        0x0B3F => IndicCategory::Matra,
        // Vowel signs II / U / UU / R / RR — post-base/below.
        0x0B40..=0x0B44 => IndicCategory::Matra,
        // Pre-base matras E / AI.
        0x0B47..=0x0B48 => IndicCategory::PreBaseMatra,
        // O / AU vowel signs are precomposed forms; their canonical
        // decomposition (U+0B47 + U+0B3E / U+0B57) starts with the
        // pre-base U+0B47 — we mirror that by classifying the
        // precomposed forms as PreBaseMatra so a cluster machine
        // operating on raw input still emits a visually-correct
        // pre-base reorder.
        0x0B4B..=0x0B4C => IndicCategory::PreBaseMatra,
        // Halant / virama.
        0x0B4D => IndicCategory::Halant,
        // Length mark (post-base).
        0x0B55..=0x0B57 => IndicCategory::Matra,
        // Additional consonants (RRA / RHA / YYA).
        0x0B5C..=0x0B5D => IndicCategory::Consonant,
        0x0B5F..=0x0B61 => IndicCategory::Vowel,
        0x0B62..=0x0B63 => IndicCategory::Matra,
        // Digits + Oriya fractions/signs.
        0x0B66..=0x0B6F => IndicCategory::Symbol,
        0x0B70..=0x0B7F => IndicCategory::Symbol,
        _ => IndicCategory::Symbol,
    }
}

/// Per-cluster shaping flags computed by [`reorder_cluster`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClusterFlags {
    /// True when the cluster started with `RA + halant + consonant`
    /// — the RA at position 0 should ultimately render as a "reph"
    /// superscript mark over the cluster end. Tamil never sets this
    /// (Tamil's RA does not form a reph). Devanagari + Bengali do.
    pub has_reph: bool,
    /// True when the cluster contained a pre-base matra that was
    /// moved to the front of the cluster.
    pub pre_base_reordered: bool,
}

/// Walk `chars` and emit `(cluster_start, cluster_end_exclusive)` byte
/// indices into `chars` for every Indic cluster of the script picked
/// by `category`. Non-Indic characters become single-character clusters
/// whose category is [`IndicCategory::Other`].
///
/// A cluster boundary starts a new cluster when:
/// - the current character is `Other` (non-Indic);
/// - the current character is `Consonant` or `Vowel` AND the previous
///   character is NOT `Halant` (a halant glues the next consonant
///   into the same cluster, forming a conjunct);
/// - the previous character was `Symbol` (danda etc. always end a
///   cluster).
///
/// Otherwise the current character extends the cluster.
///
/// Pass [`devanagari_category`], [`bengali_category`], or
/// [`tamil_category`] as `category` to drive the segmentation per
/// script. The legacy round-8 entry point [`cluster_boundaries`]
/// hard-codes [`devanagari_category`].
pub fn cluster_boundaries_with(
    chars: &[char],
    category: fn(char) -> IndicCategory,
) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    if chars.is_empty() {
        return out;
    }
    let n = chars.len();
    let mut start = 0usize;
    for i in 1..n {
        let prev = category(chars[i - 1]);
        let cur = category(chars[i]);
        let boundary = matches!(cur, IndicCategory::Other | IndicCategory::Symbol)
            || matches!(prev, IndicCategory::Other | IndicCategory::Symbol)
            || (matches!(cur, IndicCategory::Consonant | IndicCategory::Vowel)
                && !matches!(prev, IndicCategory::Halant));
        if boundary {
            out.push((start, i));
            start = i;
        }
    }
    out.push((start, n));
    out
}

/// Devanagari cluster segmenter. Convenience wrapper for
/// [`cluster_boundaries_with`] that hard-codes [`devanagari_category`]
/// — preserved for callers built against the round-8 API.
pub fn cluster_boundaries(chars: &[char]) -> Vec<(usize, usize)> {
    cluster_boundaries_with(chars, devanagari_category)
}

/// Reordering rules that describe how a single cluster is rewritten
/// from logical to visual order.
#[derive(Debug, Clone, Copy)]
pub struct ReorderRules {
    /// Lookup function that classifies a single character.
    pub category: fn(char) -> IndicCategory,
    /// Codepoint of the script's RA letter — the only consonant that
    /// can form a reph. Devanagari U+0930, Bengali U+09B0, Tamil
    /// U+0BB0 (but Tamil sets `reph_enabled = false`).
    pub ra_codepoint: char,
    /// True when this script forms a reph (Devanagari, Bengali). False
    /// for scripts where RA + halant + consonant renders in-line
    /// (Tamil, Malayalam in modern orthography).
    pub reph_enabled: bool,
}

/// Devanagari reorder rules.
pub const DEVANAGARI_RULES: ReorderRules = ReorderRules {
    category: devanagari_category,
    ra_codepoint: '\u{0930}',
    reph_enabled: true,
};

/// Bengali reorder rules.
pub const BENGALI_RULES: ReorderRules = ReorderRules {
    category: bengali_category,
    ra_codepoint: '\u{09B0}',
    reph_enabled: true,
};

/// Tamil reorder rules. `reph_enabled = false` because Tamil RA
/// does not form a superscript reph.
pub const TAMIL_RULES: ReorderRules = ReorderRules {
    category: tamil_category,
    ra_codepoint: '\u{0BB0}',
    reph_enabled: false,
};

/// Gurmukhi reorder rules. Reph is rare in modern Punjabi but the
/// flag is still set when RA + halant + consonant appears, so fonts
/// that ship a `rphf` lookup get the substitution.
pub const GURMUKHI_RULES: ReorderRules = ReorderRules {
    category: gurmukhi_category,
    ra_codepoint: '\u{0A30}',
    reph_enabled: true,
};

/// Gujarati reorder rules — Devanagari-shaped (halant-driven
/// conjuncts; reph rule on RA U+0AB0; pre-base matra U+0ABF).
pub const GUJARATI_RULES: ReorderRules = ReorderRules {
    category: gujarati_category,
    ra_codepoint: '\u{0AB0}',
    reph_enabled: true,
};

/// Telugu reorder rules. Reph forms on RA U+0C30; pre-base matras
/// U+0C46 / U+0C47 / U+0C48 reorder to the front of the cluster.
pub const TELUGU_RULES: ReorderRules = ReorderRules {
    category: telugu_category,
    ra_codepoint: '\u{0C30}',
    reph_enabled: true,
};

/// Kannada reorder rules. Reph forms on RA U+0CB0; pre-base matras
/// U+0CBF / U+0CC6 / U+0CC7 / U+0CC8 reorder to the front of the
/// cluster.
pub const KANNADA_RULES: ReorderRules = ReorderRules {
    category: kannada_category,
    ra_codepoint: '\u{0CB0}',
    reph_enabled: true,
};

/// Malayalam reorder rules. `reph_enabled = false` — modern Malayalam
/// uses chillu (independent half-form) characters U+0D7A..U+0D7F
/// instead of the historic reph rendering.
pub const MALAYALAM_RULES: ReorderRules = ReorderRules {
    category: malayalam_category,
    ra_codepoint: '\u{0D30}',
    reph_enabled: false,
};

/// Oriya reorder rules. Reph forms on RA U+0B30; pre-base matras
/// U+0B47 / U+0B48 / U+0B4B / U+0B4C reorder to the front of the
/// cluster (the precomposed o / au matras carry pre-base components).
pub const ORIYA_RULES: ReorderRules = ReorderRules {
    category: oriya_category,
    ra_codepoint: '\u{0B30}',
    reph_enabled: true,
};

/// Apply Indic cluster reordering to a single cluster using `rules`.
///
/// Returns the reordered character slice plus [`ClusterFlags`]
/// describing what was done.
///
/// Scope:
/// - **Pre-base matra** — if the cluster contains a pre-base matra
///   (any codepoint classified [`IndicCategory::PreBaseMatra`]
///   anywhere after the first consonant), move it to position 0.
///   Bengali clusters can have multiple pre-base matras in pathological
///   input; only the FIRST is moved (the others stay in place — the
///   cluster machine is tolerant rather than authoritative).
/// - **Reph detection** — if `rules.reph_enabled` is true AND the
///   cluster begins with `RA + halant + consonant`, set
///   [`ClusterFlags::has_reph`]. The actual glyph substitution is
///   wired in [`crate::face_chain`] via `Font::gsub_apply_lookup_type_1`
///   using the `rphf` feature.
pub fn reorder_cluster_with(cluster: &[char], rules: &ReorderRules) -> (Vec<char>, ClusterFlags) {
    let mut flags = ClusterFlags::default();
    if cluster.is_empty() {
        return (Vec::new(), flags);
    }
    let mut out: Vec<char> = cluster.to_vec();

    // Pre-base matra reorder. Find the FIRST pre-base matra and move
    // it to position 0.
    if let Some(matra_idx) = out
        .iter()
        .position(|&c| (rules.category)(c) == IndicCategory::PreBaseMatra)
    {
        if matra_idx > 0 {
            let matra = out.remove(matra_idx);
            out.insert(0, matra);
            flags.pre_base_reordered = true;
        }
    }

    // Reph detection. Use the original `cluster` (not `out`) so a
    // pre-base matra moved to the front doesn't mask the leading RA.
    if rules.reph_enabled
        && cluster.len() >= 3
        && cluster[0] == rules.ra_codepoint
        && (rules.category)(cluster[1]) == IndicCategory::Halant
        && (rules.category)(cluster[2]) == IndicCategory::Consonant
    {
        flags.has_reph = true;
    }

    (out, flags)
}

/// Devanagari cluster reorder. Convenience wrapper for
/// [`reorder_cluster_with`] using [`DEVANAGARI_RULES`] — preserved for
/// callers built against the round-8 API.
pub fn reorder_cluster(cluster: &[char]) -> (Vec<char>, ClusterFlags) {
    reorder_cluster_with(cluster, &DEVANAGARI_RULES)
}

/// Devanagari OpenType GSUB feature tags, in the spec-mandated
/// application order. The first 9 tags (`locl`..`cjct`) are
/// "substitution" features that reshape clusters into conjuncts and
/// half-forms; the last 6 (`init`..`haln`) are "presentation"
/// features that pick contextual variants.
pub fn devanagari_feature_tags() -> Vec<[u8; 4]> {
    vec![
        *b"locl", // language-form substitutions
        *b"ccmp", // glyph composition / decomposition
        *b"nukt", // nukta forms
        *b"akhn", // akhand ligatures (e.g. ksha, jnya)
        *b"rphf", // reph form (RA + halant → superscript)
        *b"blwf", // below-base forms
        *b"half", // half forms (consonant + halant in non-final position)
        *b"vatu", // vattu variants
        *b"cjct", // conjunct forms
        *b"init", // initial contextual variants
        *b"pres", // pre-base substitutions
        *b"abvs", // above-base substitutions
        *b"blws", // below-base substitutions
        *b"psts", // post-base substitutions
        *b"haln", // halant forms
    ]
}

/// Bengali OpenType GSUB feature tags, in the spec-mandated
/// application order. Identical shape to Devanagari — the same
/// substitution/presentation feature pipeline applies.
pub fn bengali_feature_tags() -> Vec<[u8; 4]> {
    // Bengali shares Devanagari's feature ordering one-to-one
    // (same Indic family rules in the OpenType spec).
    devanagari_feature_tags()
}

/// Tamil OpenType GSUB feature tags, in the spec-mandated application
/// order. Tamil's substitution chain is simpler than Devanagari /
/// Bengali — there's no `rphf` (no reph), no `vatu` (no vattu), no
/// `cjct` (no conjuncts in modern orthography). The remaining
/// substitution + presentation features carry over.
pub fn tamil_feature_tags() -> Vec<[u8; 4]> {
    vec![
        *b"locl", // language-form substitutions
        *b"ccmp", // glyph composition / decomposition
        *b"akhn", // akhand ligatures (rare in Tamil but present in some fonts)
        *b"half", // half forms
        *b"pref", // pre-base form (Tamil-specific: reorders the
        // pre-base component of a precomposed two-part vowel sign).
        *b"blwf", // below-base forms
        *b"pstf", // post-base forms
        *b"init", // initial contextual variants
        *b"pres", // pre-base substitutions
        *b"abvs", // above-base substitutions
        *b"blws", // below-base substitutions
        *b"psts", // post-base substitutions
        *b"haln", // halant forms
    ]
}

/// Gurmukhi feature tags. Same Devanagari-family pipeline.
pub fn gurmukhi_feature_tags() -> Vec<[u8; 4]> {
    devanagari_feature_tags()
}

/// Gujarati feature tags. Closest-to-Devanagari shape, identical
/// feature ordering.
pub fn gujarati_feature_tags() -> Vec<[u8; 4]> {
    devanagari_feature_tags()
}

/// Telugu feature tags. The Telugu/Kannada/Malayalam family adds the
/// `pref` / `pstf` / `abvf` / `abvs` per-position GSUB features on top
/// of the Devanagari list — split-vowel pre-base components flow
/// through `pref`, and the post-base components through `pstf`.
pub fn telugu_feature_tags() -> Vec<[u8; 4]> {
    vec![
        *b"locl", *b"ccmp", *b"nukt", *b"akhn", *b"rphf", *b"pref", *b"blwf", *b"half", *b"abvf",
        *b"pstf", *b"cjct", *b"init", *b"pres", *b"abvs", *b"blws", *b"psts", *b"haln",
    ]
}

/// Kannada feature tags. Same Telugu/Kannada/Malayalam family
/// (`pref` / `pstf` / `abvf` plus the standard list).
pub fn kannada_feature_tags() -> Vec<[u8; 4]> {
    telugu_feature_tags()
}

/// Malayalam feature tags. Same family as Telugu/Kannada but `rphf`
/// is omitted because Malayalam uses chillu (independent half-forms)
/// instead of reph.
pub fn malayalam_feature_tags() -> Vec<[u8; 4]> {
    vec![
        *b"locl", *b"ccmp", *b"nukt", *b"akhn", *b"pref", *b"blwf", *b"half", *b"abvf", *b"pstf",
        *b"cjct", *b"init", *b"pres", *b"abvs", *b"blws", *b"psts", *b"haln",
    ]
}

/// Oriya feature tags. Devanagari-shape feature list — adds `pref`
/// because the precomposed o / au matras decompose into pre-base
/// components.
pub fn oriya_feature_tags() -> Vec<[u8; 4]> {
    vec![
        *b"locl", *b"ccmp", *b"nukt", *b"akhn", *b"rphf", *b"pref", *b"blwf", *b"half", *b"vatu",
        *b"cjct", *b"init", *b"pres", *b"abvs", *b"blws", *b"psts", *b"haln",
    ]
}

/// OpenType script tags for the Indic scripts we shape. Each tuple
/// returns `(modern_tag, legacy_tag)` — modern Indic2 tags
/// (`dev2` / `bng2` / `tml2` / `gur2` / `gjr2` / `tel2` / `knd2` /
/// `mlm2` / `ory2`) carry the up-to-date feature lookups in most
/// fonts; legacy v1 tags ship the pre-2005 lookups for compatibility
/// with older shapers.
///
/// Use [`script_indic_tags`] to fetch the pair for a given script.
pub fn script_indic_tags(script: super::arabic::Script) -> Option<([u8; 4], [u8; 4])> {
    match script {
        super::arabic::Script::Devanagari => Some((*b"dev2", *b"deva")),
        super::arabic::Script::Bengali => Some((*b"bng2", *b"beng")),
        super::arabic::Script::Tamil => Some((*b"tml2", *b"taml")),
        super::arabic::Script::Gurmukhi => Some((*b"gur2", *b"guru")),
        super::arabic::Script::Gujarati => Some((*b"gjr2", *b"gujr")),
        super::arabic::Script::Telugu => Some((*b"tel2", *b"telu")),
        super::arabic::Script::Kannada => Some((*b"knd2", *b"knda")),
        super::arabic::Script::Malayalam => Some((*b"mlm2", *b"mlym")),
        super::arabic::Script::Oriya => Some((*b"ory2", *b"orya")),
        _ => None,
    }
}

#[cfg(test)]
#[allow(non_snake_case)] // tests reference Unicode codepoint literals
mod tests {
    use super::*;

    // ---------- Devanagari (round 8 baseline tests) ----------

    #[test]
    fn devanagari_category_lookup_returns_consonant_for_ka_U_0915() {
        assert_eq!(devanagari_category('\u{0915}'), IndicCategory::Consonant);
    }

    #[test]
    fn devanagari_category_lookup_returns_halant_for_U_094D() {
        assert_eq!(devanagari_category('\u{094D}'), IndicCategory::Halant);
    }

    #[test]
    fn devanagari_category_lookup_returns_pre_base_matra_for_U_093F() {
        assert_eq!(devanagari_category('\u{093F}'), IndicCategory::PreBaseMatra);
    }

    #[test]
    fn devanagari_category_classifies_vowel_a_as_vowel() {
        assert_eq!(devanagari_category('\u{0905}'), IndicCategory::Vowel);
    }

    #[test]
    fn devanagari_category_classifies_anusvara_as_bindu() {
        assert_eq!(devanagari_category('\u{0902}'), IndicCategory::Bindu);
    }

    #[test]
    fn devanagari_category_classifies_nukta_as_nukta() {
        assert_eq!(devanagari_category('\u{093C}'), IndicCategory::Nukta);
    }

    #[test]
    fn devanagari_category_classifies_post_base_matra_aa_as_matra() {
        assert_eq!(devanagari_category('\u{093E}'), IndicCategory::Matra);
    }

    #[test]
    fn devanagari_category_classifies_danda_as_symbol() {
        assert_eq!(devanagari_category('\u{0964}'), IndicCategory::Symbol);
    }

    #[test]
    fn devanagari_category_returns_other_for_latin_a() {
        assert_eq!(devanagari_category('A'), IndicCategory::Other);
    }

    #[test]
    fn script_of_recognises_devanagari_block() {
        use super::super::arabic::{script_of, Script};
        assert_eq!(script_of('\u{0915}'), Script::Devanagari);
        assert_eq!(script_of('\u{094D}'), Script::Devanagari);
        assert_eq!(script_of('\u{097F}'), Script::Devanagari);
    }

    #[test]
    fn script_of_still_classifies_arabic_and_latin_correctly() {
        use super::super::arabic::{script_of, Script};
        assert_eq!(script_of('\u{0627}'), Script::Arabic);
        assert_eq!(script_of('A'), Script::Other);
    }

    #[test]
    fn pre_base_matra_reorders_before_base_consonant() {
        let cluster = ['\u{0915}', '\u{093F}'];
        let (out, flags) = reorder_cluster(&cluster);
        assert_eq!(out, vec!['\u{093F}', '\u{0915}']);
        assert!(flags.pre_base_reordered);
        assert!(!flags.has_reph);
    }

    #[test]
    fn pre_base_matra_reorders_in_conjunct_cluster() {
        let cluster = ['\u{0915}', '\u{094D}', '\u{0937}', '\u{093F}'];
        let (out, flags) = reorder_cluster(&cluster);
        assert_eq!(out, vec!['\u{093F}', '\u{0915}', '\u{094D}', '\u{0937}']);
        assert!(flags.pre_base_reordered);
    }

    #[test]
    fn reph_formation_at_cluster_start_marks_RA_for_superscript() {
        let cluster = ['\u{0930}', '\u{094D}', '\u{0915}'];
        let (out, flags) = reorder_cluster(&cluster);
        assert_eq!(out, vec!['\u{0930}', '\u{094D}', '\u{0915}']);
        assert!(flags.has_reph);
        assert!(!flags.pre_base_reordered);
    }

    #[test]
    fn reph_with_pre_base_matra_combines_both_flags() {
        let cluster = ['\u{0930}', '\u{094D}', '\u{0915}', '\u{093F}'];
        let (out, flags) = reorder_cluster(&cluster);
        assert_eq!(out, vec!['\u{093F}', '\u{0930}', '\u{094D}', '\u{0915}']);
        assert!(flags.has_reph);
        assert!(flags.pre_base_reordered);
    }

    #[test]
    fn cluster_without_reph_consonant_does_not_set_flag() {
        let cluster = ['\u{0915}', '\u{094D}', '\u{0937}'];
        let (_out, flags) = reorder_cluster(&cluster);
        assert!(!flags.has_reph);
    }

    #[test]
    fn cluster_boundary_starts_new_cluster_at_consonant_after_vowel() {
        let chars = ['\u{0915}', '\u{093E}', '\u{0915}'];
        let bounds = cluster_boundaries(&chars);
        assert_eq!(bounds, vec![(0, 2), (2, 3)]);
    }

    #[test]
    fn cluster_boundary_keeps_conjunct_in_one_cluster() {
        let chars = ['\u{0915}', '\u{094D}', '\u{0937}'];
        let bounds = cluster_boundaries(&chars);
        assert_eq!(bounds, vec![(0, 3)]);
    }

    #[test]
    fn cluster_boundary_breaks_at_danda_symbol() {
        let chars = ['\u{0915}', '\u{0964}', '\u{0915}'];
        let bounds = cluster_boundaries(&chars);
        assert_eq!(bounds, vec![(0, 1), (1, 2), (2, 3)]);
    }

    #[test]
    fn cluster_boundary_breaks_at_non_indic_codepoint() {
        let chars = ['\u{0915}', ' ', '\u{0915}'];
        let bounds = cluster_boundaries(&chars);
        assert_eq!(bounds, vec![(0, 1), (1, 2), (2, 3)]);
    }

    #[test]
    fn cluster_boundary_handles_empty_input() {
        let bounds = cluster_boundaries(&[]);
        assert!(bounds.is_empty());
    }

    #[test]
    fn cluster_boundary_single_consonant_is_one_cluster() {
        let chars = ['\u{0915}'];
        let bounds = cluster_boundaries(&chars);
        assert_eq!(bounds, vec![(0, 1)]);
    }

    #[test]
    fn devanagari_feature_tags_are_in_canonical_order() {
        let tags = devanagari_feature_tags();
        assert_eq!(&tags[0], b"locl");
        assert_eq!(&tags[1], b"ccmp");
        assert_eq!(&tags[2], b"nukt");
        assert_eq!(&tags[3], b"akhn");
        assert_eq!(&tags[4], b"rphf");
        assert_eq!(tags.last(), Some(b"haln"));
    }

    #[test]
    fn empty_cluster_reorder_returns_empty() {
        let (out, flags) = reorder_cluster(&[]);
        assert!(out.is_empty());
        assert_eq!(flags, ClusterFlags::default());
    }

    #[test]
    fn single_consonant_cluster_does_not_reorder() {
        let cluster = ['\u{0915}'];
        let (out, flags) = reorder_cluster(&cluster);
        assert_eq!(out, vec!['\u{0915}']);
        assert!(!flags.pre_base_reordered);
        assert!(!flags.has_reph);
    }

    #[test]
    fn two_clusters_with_pre_base_matras_each_reorder_independently() {
        let chars = ['\u{0915}', '\u{093F}', '\u{0915}', '\u{093F}'];
        let bounds = cluster_boundaries(&chars);
        assert_eq!(bounds, vec![(0, 2), (2, 4)]);
        for (s, e) in bounds {
            let (out, flags) = reorder_cluster(&chars[s..e]);
            assert_eq!(out, vec!['\u{093F}', '\u{0915}']);
            assert!(flags.pre_base_reordered);
        }
    }

    // ---------- Bengali (round 10) ----------

    #[test]
    fn bengali_category_classifies_ka_as_consonant() {
        // U+0995 BENGALI LETTER KA.
        assert_eq!(bengali_category('\u{0995}'), IndicCategory::Consonant);
    }

    #[test]
    fn bengali_category_classifies_ra_as_consonant() {
        // U+09B0 BENGALI LETTER RA.
        assert_eq!(bengali_category('\u{09B0}'), IndicCategory::Consonant);
    }

    #[test]
    fn bengali_category_classifies_halant_as_halant() {
        // U+09CD BENGALI SIGN VIRAMA (hashanta).
        assert_eq!(bengali_category('\u{09CD}'), IndicCategory::Halant);
    }

    #[test]
    fn bengali_category_classifies_nukta_as_nukta() {
        // U+09BC BENGALI SIGN NUKTA.
        assert_eq!(bengali_category('\u{09BC}'), IndicCategory::Nukta);
    }

    #[test]
    fn bengali_category_pre_base_matras_i_e_ai() {
        // U+09BF, U+09C7, U+09C8 — ALL pre-base in Bengali.
        assert_eq!(bengali_category('\u{09BF}'), IndicCategory::PreBaseMatra);
        assert_eq!(bengali_category('\u{09C7}'), IndicCategory::PreBaseMatra);
        assert_eq!(bengali_category('\u{09C8}'), IndicCategory::PreBaseMatra);
    }

    #[test]
    fn bengali_category_classifies_aa_matra_as_matra() {
        // U+09BE BENGALI VOWEL SIGN AA — post-base matra.
        assert_eq!(bengali_category('\u{09BE}'), IndicCategory::Matra);
    }

    #[test]
    fn bengali_category_classifies_anusvara_as_bindu() {
        // U+0982 BENGALI SIGN ANUSVARA.
        assert_eq!(bengali_category('\u{0982}'), IndicCategory::Bindu);
    }

    #[test]
    fn bengali_category_classifies_independent_vowel_a_as_vowel() {
        // U+0985 BENGALI LETTER A.
        assert_eq!(bengali_category('\u{0985}'), IndicCategory::Vowel);
    }

    #[test]
    fn bengali_category_returns_other_for_devanagari_codepoint() {
        // Devanagari is OUT of the Bengali block.
        assert_eq!(bengali_category('\u{0915}'), IndicCategory::Other);
    }

    #[test]
    fn bengali_pre_base_matra_i_reorders_before_base() {
        // BENGALI KA + sign-i → sign-i + KA.
        let cluster = ['\u{0995}', '\u{09BF}'];
        let (out, flags) = reorder_cluster_with(&cluster, &BENGALI_RULES);
        assert_eq!(out, vec!['\u{09BF}', '\u{0995}']);
        assert!(flags.pre_base_reordered);
        assert!(!flags.has_reph);
    }

    #[test]
    fn bengali_pre_base_matra_e_reorders_before_base() {
        // BENGALI KA + sign-e → sign-e + KA.
        let cluster = ['\u{0995}', '\u{09C7}'];
        let (out, flags) = reorder_cluster_with(&cluster, &BENGALI_RULES);
        assert_eq!(out, vec!['\u{09C7}', '\u{0995}']);
        assert!(flags.pre_base_reordered);
    }

    #[test]
    fn bengali_pre_base_matra_ai_reorders_before_base() {
        // BENGALI KA + sign-ai → sign-ai + KA.
        let cluster = ['\u{0995}', '\u{09C8}'];
        let (out, flags) = reorder_cluster_with(&cluster, &BENGALI_RULES);
        assert_eq!(out, vec!['\u{09C8}', '\u{0995}']);
        assert!(flags.pre_base_reordered);
    }

    #[test]
    fn bengali_reph_formation_marks_RA_for_superscript() {
        // BENGALI RA + halant + KA → reph + KA.
        let cluster = ['\u{09B0}', '\u{09CD}', '\u{0995}'];
        let (out, flags) = reorder_cluster_with(&cluster, &BENGALI_RULES);
        assert_eq!(out, vec!['\u{09B0}', '\u{09CD}', '\u{0995}']);
        assert!(flags.has_reph);
    }

    #[test]
    fn bengali_conjunct_keeps_in_one_cluster() {
        // BENGALI KA + halant + SHA → conjunct (single cluster).
        let chars = ['\u{0995}', '\u{09CD}', '\u{09B7}'];
        let bounds = cluster_boundaries_with(&chars, bengali_category);
        assert_eq!(bounds, vec![(0, 3)]);
    }

    // ---------- Tamil (round 10) ----------

    #[test]
    fn tamil_category_classifies_ka_as_consonant() {
        // U+0B95 TAMIL LETTER KA.
        assert_eq!(tamil_category('\u{0B95}'), IndicCategory::Consonant);
    }

    #[test]
    fn tamil_category_classifies_ra_as_consonant() {
        // U+0BB0 TAMIL LETTER RA.
        assert_eq!(tamil_category('\u{0BB0}'), IndicCategory::Consonant);
    }

    #[test]
    fn tamil_category_classifies_pulli_as_halant() {
        // U+0BCD TAMIL SIGN VIRAMA (pulli).
        assert_eq!(tamil_category('\u{0BCD}'), IndicCategory::Halant);
    }

    #[test]
    fn tamil_category_pre_base_matras_e_ee_ai() {
        // U+0BC6 (e), U+0BC7 (ee), U+0BC8 (ai) — pre-base.
        assert_eq!(tamil_category('\u{0BC6}'), IndicCategory::PreBaseMatra);
        assert_eq!(tamil_category('\u{0BC7}'), IndicCategory::PreBaseMatra);
        assert_eq!(tamil_category('\u{0BC8}'), IndicCategory::PreBaseMatra);
    }

    #[test]
    fn tamil_category_classifies_aa_matra_as_matra() {
        // U+0BBE TAMIL VOWEL SIGN AA — post-base.
        assert_eq!(tamil_category('\u{0BBE}'), IndicCategory::Matra);
    }

    #[test]
    fn tamil_category_classifies_anusvara_as_bindu() {
        // U+0B82 TAMIL SIGN ANUSVARA.
        assert_eq!(tamil_category('\u{0B82}'), IndicCategory::Bindu);
    }

    #[test]
    fn tamil_category_classifies_independent_vowel_a_as_vowel() {
        // U+0B85 TAMIL LETTER A.
        assert_eq!(tamil_category('\u{0B85}'), IndicCategory::Vowel);
    }

    #[test]
    fn tamil_category_returns_other_for_devanagari_codepoint() {
        assert_eq!(tamil_category('\u{0915}'), IndicCategory::Other);
    }

    #[test]
    fn tamil_pre_base_matra_e_reorders_before_base() {
        // TAMIL KA + sign-e → sign-e + KA.
        let cluster = ['\u{0B95}', '\u{0BC6}'];
        let (out, flags) = reorder_cluster_with(&cluster, &TAMIL_RULES);
        assert_eq!(out, vec!['\u{0BC6}', '\u{0B95}']);
        assert!(flags.pre_base_reordered);
    }

    #[test]
    fn tamil_RA_plus_halant_does_NOT_set_reph_flag() {
        // Tamil RA + pulli + KA — Tamil never forms a reph.
        let cluster = ['\u{0BB0}', '\u{0BCD}', '\u{0B95}'];
        let (_out, flags) = reorder_cluster_with(&cluster, &TAMIL_RULES);
        assert!(!flags.has_reph, "Tamil reph_enabled is false");
    }

    #[test]
    fn tamil_cluster_boundary_keeps_pulli_chain_in_one_cluster() {
        // KA + pulli + KA → conjunct-like cluster.
        let chars = ['\u{0B95}', '\u{0BCD}', '\u{0B95}'];
        let bounds = cluster_boundaries_with(&chars, tamil_category);
        assert_eq!(bounds, vec![(0, 3)]);
    }

    #[test]
    fn tamil_feature_tags_omit_rphf_and_cjct() {
        let tags = tamil_feature_tags();
        assert!(!tags.contains(b"rphf"), "Tamil has no reph feature");
        assert!(!tags.contains(b"cjct"), "Tamil has no conjunct feature");
        assert!(!tags.contains(b"vatu"), "Tamil has no vattu feature");
        // Tamil-specific tag.
        assert!(tags.contains(b"pref"), "Tamil emits the pref feature");
    }

    #[test]
    fn bengali_feature_tags_match_devanagari_shape() {
        assert_eq!(bengali_feature_tags(), devanagari_feature_tags());
    }

    #[test]
    fn script_indic_tags_returns_modern_and_legacy_pair_for_devanagari() {
        use super::super::arabic::Script;
        let pair = script_indic_tags(Script::Devanagari);
        assert_eq!(pair, Some((*b"dev2", *b"deva")));
    }

    #[test]
    fn script_indic_tags_returns_pair_for_bengali_and_tamil() {
        use super::super::arabic::Script;
        assert_eq!(
            script_indic_tags(Script::Bengali),
            Some((*b"bng2", *b"beng"))
        );
        assert_eq!(script_indic_tags(Script::Tamil), Some((*b"tml2", *b"taml")));
    }

    #[test]
    fn script_indic_tags_returns_none_for_arabic_or_other() {
        use super::super::arabic::Script;
        assert_eq!(script_indic_tags(Script::Arabic), None);
        assert_eq!(script_indic_tags(Script::Other), None);
    }

    // ---------- Gurmukhi (round 11) ----------

    #[test]
    fn gurmukhi_category_classifies_ka_as_consonant() {
        // U+0A15 GURMUKHI LETTER KA.
        assert_eq!(gurmukhi_category('\u{0A15}'), IndicCategory::Consonant);
    }

    #[test]
    fn gurmukhi_category_classifies_halant_as_halant() {
        // U+0A4D GURMUKHI SIGN VIRAMA.
        assert_eq!(gurmukhi_category('\u{0A4D}'), IndicCategory::Halant);
    }

    #[test]
    fn gurmukhi_category_classifies_pre_base_matra_i() {
        // U+0A3F GURMUKHI VOWEL SIGN I — pre-base.
        assert_eq!(gurmukhi_category('\u{0A3F}'), IndicCategory::PreBaseMatra);
    }

    #[test]
    fn gurmukhi_pre_base_matra_i_reorders_before_base() {
        // KA + sign-i → sign-i + KA.
        let cluster = ['\u{0A15}', '\u{0A3F}'];
        let (out, flags) = reorder_cluster_with(&cluster, &GURMUKHI_RULES);
        assert_eq!(out, vec!['\u{0A3F}', '\u{0A15}']);
        assert!(flags.pre_base_reordered);
    }

    #[test]
    fn gurmukhi_reph_marks_RA_for_superscript() {
        // RA + halant + KA — Gurmukhi flags reph (rare in modern usage
        // but fonts that ship the lookup pick it up).
        let cluster = ['\u{0A30}', '\u{0A4D}', '\u{0A15}'];
        let (_out, flags) = reorder_cluster_with(&cluster, &GURMUKHI_RULES);
        assert!(flags.has_reph);
    }

    // ---------- Gujarati (round 11) ----------

    #[test]
    fn gujarati_category_classifies_ka_as_consonant() {
        // U+0A95 GUJARATI LETTER KA.
        assert_eq!(gujarati_category('\u{0A95}'), IndicCategory::Consonant);
    }

    #[test]
    fn gujarati_category_classifies_halant_as_halant() {
        // U+0ACD GUJARATI SIGN VIRAMA.
        assert_eq!(gujarati_category('\u{0ACD}'), IndicCategory::Halant);
    }

    #[test]
    fn gujarati_category_classifies_pre_base_matra_i() {
        // U+0ABF GUJARATI VOWEL SIGN I — pre-base.
        assert_eq!(gujarati_category('\u{0ABF}'), IndicCategory::PreBaseMatra);
    }

    #[test]
    fn gujarati_pre_base_matra_i_reorders_before_base() {
        // KA + sign-i → sign-i + KA.
        let cluster = ['\u{0A95}', '\u{0ABF}'];
        let (out, flags) = reorder_cluster_with(&cluster, &GUJARATI_RULES);
        assert_eq!(out, vec!['\u{0ABF}', '\u{0A95}']);
        assert!(flags.pre_base_reordered);
    }

    #[test]
    fn gujarati_reph_marks_RA_for_superscript() {
        // RA U+0AB0 + halant + KA → reph.
        let cluster = ['\u{0AB0}', '\u{0ACD}', '\u{0A95}'];
        let (_out, flags) = reorder_cluster_with(&cluster, &GUJARATI_RULES);
        assert!(flags.has_reph);
    }

    // ---------- Telugu (round 11) ----------

    #[test]
    fn telugu_category_classifies_ka_as_consonant() {
        // U+0C15 TELUGU LETTER KA.
        assert_eq!(telugu_category('\u{0C15}'), IndicCategory::Consonant);
    }

    #[test]
    fn telugu_category_classifies_halant_as_halant() {
        // U+0C4D TELUGU SIGN VIRAMA.
        assert_eq!(telugu_category('\u{0C4D}'), IndicCategory::Halant);
    }

    #[test]
    fn telugu_pre_base_matras_e_ee_ai() {
        // U+0C46 / U+0C47 / U+0C48 — pre-base.
        assert_eq!(telugu_category('\u{0C46}'), IndicCategory::PreBaseMatra);
        assert_eq!(telugu_category('\u{0C47}'), IndicCategory::PreBaseMatra);
        assert_eq!(telugu_category('\u{0C48}'), IndicCategory::PreBaseMatra);
    }

    #[test]
    fn telugu_pre_base_matra_e_reorders_before_base() {
        // KA + sign-e → sign-e + KA.
        let cluster = ['\u{0C15}', '\u{0C46}'];
        let (out, flags) = reorder_cluster_with(&cluster, &TELUGU_RULES);
        assert_eq!(out, vec!['\u{0C46}', '\u{0C15}']);
        assert!(flags.pre_base_reordered);
    }

    #[test]
    fn telugu_reph_marks_RA_for_superscript() {
        // RA U+0C30 + halant + KA → reph.
        let cluster = ['\u{0C30}', '\u{0C4D}', '\u{0C15}'];
        let (_out, flags) = reorder_cluster_with(&cluster, &TELUGU_RULES);
        assert!(flags.has_reph);
    }

    // ---------- Kannada (round 11) ----------

    #[test]
    fn kannada_category_classifies_ka_as_consonant() {
        // U+0C95 KANNADA LETTER KA.
        assert_eq!(kannada_category('\u{0C95}'), IndicCategory::Consonant);
    }

    #[test]
    fn kannada_category_classifies_halant_as_halant() {
        // U+0CCD KANNADA SIGN VIRAMA.
        assert_eq!(kannada_category('\u{0CCD}'), IndicCategory::Halant);
    }

    #[test]
    fn kannada_pre_base_matras_e_ee_ai() {
        // U+0CC6 / U+0CC7 / U+0CC8 — pre-base.
        assert_eq!(kannada_category('\u{0CC6}'), IndicCategory::PreBaseMatra);
        assert_eq!(kannada_category('\u{0CC7}'), IndicCategory::PreBaseMatra);
        assert_eq!(kannada_category('\u{0CC8}'), IndicCategory::PreBaseMatra);
    }

    #[test]
    fn kannada_pre_base_matra_e_reorders_before_base() {
        let cluster = ['\u{0C95}', '\u{0CC6}'];
        let (out, flags) = reorder_cluster_with(&cluster, &KANNADA_RULES);
        assert_eq!(out, vec!['\u{0CC6}', '\u{0C95}']);
        assert!(flags.pre_base_reordered);
    }

    #[test]
    fn kannada_reph_marks_RA_for_superscript() {
        // RA U+0CB0 + halant + KA → reph.
        let cluster = ['\u{0CB0}', '\u{0CCD}', '\u{0C95}'];
        let (_out, flags) = reorder_cluster_with(&cluster, &KANNADA_RULES);
        assert!(flags.has_reph);
    }

    // ---------- Malayalam (round 11) ----------

    #[test]
    fn malayalam_category_classifies_ka_as_consonant() {
        // U+0D15 MALAYALAM LETTER KA.
        assert_eq!(malayalam_category('\u{0D15}'), IndicCategory::Consonant);
    }

    #[test]
    fn malayalam_category_classifies_halant_as_halant() {
        // U+0D4D MALAYALAM SIGN VIRAMA.
        assert_eq!(malayalam_category('\u{0D4D}'), IndicCategory::Halant);
    }

    #[test]
    fn malayalam_pre_base_matras_e_ee_ai() {
        // U+0D46 / U+0D47 / U+0D48 — pre-base.
        assert_eq!(malayalam_category('\u{0D46}'), IndicCategory::PreBaseMatra);
        assert_eq!(malayalam_category('\u{0D47}'), IndicCategory::PreBaseMatra);
        assert_eq!(malayalam_category('\u{0D48}'), IndicCategory::PreBaseMatra);
    }

    #[test]
    fn malayalam_chillu_classified_as_consonant() {
        // U+0D7A..U+0D7F — chillu independent half-forms.
        for cp in 0x0D7A..=0x0D7F {
            let ch = char::from_u32(cp).unwrap();
            assert_eq!(
                malayalam_category(ch),
                IndicCategory::Consonant,
                "chillu U+{cp:04X} should be Consonant"
            );
        }
    }

    #[test]
    fn malayalam_pre_base_matra_e_reorders_before_base() {
        let cluster = ['\u{0D15}', '\u{0D46}'];
        let (out, flags) = reorder_cluster_with(&cluster, &MALAYALAM_RULES);
        assert_eq!(out, vec!['\u{0D46}', '\u{0D15}']);
        assert!(flags.pre_base_reordered);
    }

    #[test]
    fn malayalam_RA_plus_halant_does_NOT_set_reph_flag() {
        // Modern Malayalam — chillu replaces reph.
        let cluster = ['\u{0D30}', '\u{0D4D}', '\u{0D15}'];
        let (_out, flags) = reorder_cluster_with(&cluster, &MALAYALAM_RULES);
        assert!(!flags.has_reph);
    }

    // ---------- Oriya (round 11) ----------

    #[test]
    fn oriya_category_classifies_ka_as_consonant() {
        // U+0B15 ORIYA LETTER KA.
        assert_eq!(oriya_category('\u{0B15}'), IndicCategory::Consonant);
    }

    #[test]
    fn oriya_category_classifies_halant_as_halant() {
        // U+0B4D ORIYA SIGN VIRAMA.
        assert_eq!(oriya_category('\u{0B4D}'), IndicCategory::Halant);
    }

    #[test]
    fn oriya_pre_base_matras_e_ai_o_au() {
        // U+0B47 / U+0B48 / U+0B4B / U+0B4C — pre-base
        // (precomposed o / au carry pre-base components).
        assert_eq!(oriya_category('\u{0B47}'), IndicCategory::PreBaseMatra);
        assert_eq!(oriya_category('\u{0B48}'), IndicCategory::PreBaseMatra);
        assert_eq!(oriya_category('\u{0B4B}'), IndicCategory::PreBaseMatra);
        assert_eq!(oriya_category('\u{0B4C}'), IndicCategory::PreBaseMatra);
    }

    #[test]
    fn oriya_pre_base_matra_e_reorders_before_base() {
        let cluster = ['\u{0B15}', '\u{0B47}'];
        let (out, flags) = reorder_cluster_with(&cluster, &ORIYA_RULES);
        assert_eq!(out, vec!['\u{0B47}', '\u{0B15}']);
        assert!(flags.pre_base_reordered);
    }

    #[test]
    fn oriya_reph_marks_RA_for_superscript() {
        // RA U+0B30 + halant + KA → reph.
        let cluster = ['\u{0B30}', '\u{0B4D}', '\u{0B15}'];
        let (_out, flags) = reorder_cluster_with(&cluster, &ORIYA_RULES);
        assert!(flags.has_reph);
    }

    // ---------- script_indic_tags expansions ----------

    #[test]
    fn script_indic_tags_returns_pair_for_all_round11_scripts() {
        use super::super::arabic::Script;
        assert_eq!(
            script_indic_tags(Script::Gurmukhi),
            Some((*b"gur2", *b"guru"))
        );
        assert_eq!(
            script_indic_tags(Script::Gujarati),
            Some((*b"gjr2", *b"gujr"))
        );
        assert_eq!(
            script_indic_tags(Script::Telugu),
            Some((*b"tel2", *b"telu"))
        );
        assert_eq!(
            script_indic_tags(Script::Kannada),
            Some((*b"knd2", *b"knda"))
        );
        assert_eq!(
            script_indic_tags(Script::Malayalam),
            Some((*b"mlm2", *b"mlym"))
        );
        assert_eq!(script_indic_tags(Script::Oriya), Some((*b"ory2", *b"orya")));
    }

    #[test]
    fn telugu_feature_tags_includes_pref_pstf_abvf_position_features() {
        let tags = telugu_feature_tags();
        // Cluster-position-aware GSUB features wired in round 11.
        assert!(tags.contains(b"pref"), "Telugu emits pref");
        assert!(tags.contains(b"pstf"), "Telugu emits pstf");
        assert!(tags.contains(b"abvf"), "Telugu emits abvf");
        // Telugu still has reph.
        assert!(tags.contains(b"rphf"), "Telugu emits rphf");
    }

    #[test]
    fn malayalam_feature_tags_omits_rphf_keeps_position_features() {
        let tags = malayalam_feature_tags();
        assert!(!tags.contains(b"rphf"), "Malayalam has no rphf (chillu)");
        assert!(tags.contains(b"pref"));
        assert!(tags.contains(b"pstf"));
        assert!(tags.contains(b"abvf"));
        assert!(tags.contains(b"blwf"));
    }

    #[test]
    fn gujarati_feature_tags_match_devanagari_shape() {
        assert_eq!(gujarati_feature_tags(), devanagari_feature_tags());
    }

    #[test]
    fn gurmukhi_feature_tags_match_devanagari_shape() {
        assert_eq!(gurmukhi_feature_tags(), devanagari_feature_tags());
    }
}
