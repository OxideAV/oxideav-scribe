//! Round 283 — UAX #9 data-driven property lookups. As of round 319
//! `Bidi_Class` and `Bidi_Mirroring_Glyph` are provided by the `intl`
//! crate's compiled UCD tables; `Bidi_Paired_Bracket` is still read
//! from the `BidiBrackets.txt` snapshot under
//! `docs/text/unicode-bidi/`, vendored into `src/bidi/`. These tests
//! assert the property *behaviour* (specific code-point → class /
//! mirror / bracket), which is unchanged by the data-source switch.
//!
//! Exercises the three public lookups through the crate root —
//! [`oxideav_scribe::bidi_class`] (full per-code-point `Bidi_Class`
//! incl. the `@missing` unassigned-code-point defaults),
//! [`oxideav_scribe::paired_bracket`] (the normative
//! `Bidi_Paired_Bracket` / `Bidi_Paired_Bracket_Type` table consumed
//! by BD14 / BD15 / BD16 / N0), and
//! [`oxideav_scribe::mirrored_glyph`] (the `Bidi_Mirroring_Glyph`
//! table consumed by L4) — plus the BD16 close-branch "U+3009 and
//! U+232A are treated as equivalent" canonical-equivalence clause.
//!
//! Provenance: every expected value below is transcribed from the
//! Unicode 16.0 UCD snapshots and the UAX #9 Revision 50 rule
//! statements staged under `docs/text/unicode-bidi/`.

use oxideav_scribe::{
    apply_mirroring, bidi_class, bracket_pairs, mirrored_glyph, paired_bracket,
    process_paragraph_with_brackets, BidiClass, BracketKind,
};

// --- Bidi_Class: explicit DerivedBidiClass.txt assignments ----------

/// Strong / weak classes far outside the old hand-mapped block list.
#[test]
fn r283_class_explicit_assignments_across_planes() {
    // 0800..0815 ; R  — Samaritan letters.
    assert_eq!(bidi_class('\u{0800}'), BidiClass::R);
    // 0840..0858 ; R  — Mandaic letters.
    assert_eq!(bidi_class('\u{0840}'), BidiClass::R);
    // 08A0..08C8 ; AL — Arabic Extended-A letters.
    assert_eq!(bidi_class('\u{08A0}'), BidiClass::AL);
    // 10E60..10E7E ; AN — Rumi digits (plane 1).
    assert_eq!(bidi_class('\u{10E60}'), BidiClass::AN);
    // 1D7CE..1D7FF ; EN — mathematical digits (plane 1).
    assert_eq!(bidi_class('\u{1D7CE}'), BidiClass::EN);
    // 1EC71..1ECAB ; AL — Indic Siyaq numbers (plane 1).
    assert_eq!(bidi_class('\u{1EC71}'), BidiClass::AL);
    // 2212 ; ES — MINUS SIGN.
    assert_eq!(bidi_class('\u{2212}'), BidiClass::ES);
    // 066B..066C ; AN — Arabic decimal / thousands separators.
    assert_eq!(bidi_class('\u{066B}'), BidiClass::AN);
    // 20A0..20C0 ; ET — currency signs incl. 20AC EURO SIGN.
    assert_eq!(bidi_class('\u{20AC}'), BidiClass::ET);
    // 0021..0022 ; ON — ASCII punctuation the old table defaulted
    // to L now resolves per the data file.
    assert_eq!(bidi_class('!'), BidiClass::ON);
    assert_eq!(bidi_class('"'), BidiClass::ON);
    // 0F3A / 0F3B ; ON — the Tibetan paired brackets.
    assert_eq!(bidi_class('\u{0F3A}'), BidiClass::ON);
    assert_eq!(bidi_class('\u{0F3B}'), BidiClass::ON);
}

/// Noncharacters are Boundary_Neutral (the file's "unassigned code
/// points that default to BN have ... Noncharacter_Code_Point" rule
/// is materialised as explicit `Cn` data lines).
#[test]
fn r283_class_noncharacters_are_bn() {
    assert_eq!(bidi_class('\u{FDD0}'), BidiClass::BN);
    assert_eq!(bidi_class('\u{FFFE}'), BidiClass::BN);
    assert_eq!(bidi_class('\u{FFFF}'), BidiClass::BN);
}

// --- Bidi_Class: @missing defaults for unassigned code points ------

/// `# @missing: 0590..05FF; Right_To_Left` — unassigned code points
/// in the Hebrew block stay strongly RTL (UAX #9 §3.2: "Unassigned
/// characters are given strong types in the algorithm. This is an
/// explicit exception to the general Unicode conformance
/// requirements with respect to unassigned characters.").
#[test]
fn r283_class_missing_defaults_for_rtl_blocks() {
    // 05D0..05EA ; R is the explicit Hebrew-letter run; 05EB..05EE
    // are unassigned and take the block @missing default R.
    assert_eq!(bidi_class('\u{05EB}'), BidiClass::R);
    // @missing: 0600..07BF; Arabic_Letter — unassigned gaps in the
    // Arabic blocks default to AL (U+074B/U+074C is an unassigned
    // gap between Syriac and Arabic Supplement).
    assert_eq!(bidi_class('\u{074B}'), BidiClass::AL);
    // @missing: 1E800..1EC6F; Right_To_Left — plane-1 RTL default.
    assert_eq!(bidi_class('\u{1EC00}'), BidiClass::R);
    // @missing: 1EE00..1EEFF; Arabic_Letter — Arabic Mathematical
    // Alphabetic Symbols unassigned gaps default to AL.
    assert_eq!(bidi_class('\u{1EE04}'), BidiClass::AL);
}

/// `# @missing: 20A0..20CF; European_Terminator` — unassigned code
/// points in the Currency Symbols block default to ET.
#[test]
fn r283_class_missing_default_currency_block_is_et() {
    // 20A0..20C0 are assigned (ET); 20C1..20CF are unassigned and
    // take the block @missing default ET.
    assert_eq!(bidi_class('\u{20C1}'), BidiClass::ET);
    assert_eq!(bidi_class('\u{20CF}'), BidiClass::ET);
}

/// The global default: "All code points not explicitly listed for
/// Bidi_Class have the value Left_To_Right (L)".
#[test]
fn r283_class_global_default_is_l() {
    assert_eq!(bidi_class('\u{0391}'), BidiClass::L); // Greek alpha
    assert_eq!(bidi_class('\u{0410}'), BidiClass::L); // Cyrillic A
    assert_eq!(bidi_class('\u{4E2D}'), BidiClass::L); // CJK ideograph
    assert_eq!(bidi_class('\u{AC00}'), BidiClass::L); // Hangul syllable
}

// --- Bidi_Paired_Bracket / Bidi_Paired_Bracket_Type ----------------

/// Non-ASCII entries from the normative BidiBrackets.txt table (BD14
/// / BD15): Tibetan, square-bracket-with-quill, CJK, fullwidth.
#[test]
fn r283_paired_bracket_full_table_entries() {
    for (open, close) in [
        ('\u{0F3A}', '\u{0F3B}'), // Tibetan gug rtags
        ('\u{2045}', '\u{2046}'), // square bracket with quill
        ('\u{2329}', '\u{232A}'), // (deprecated) angle brackets
        ('\u{3008}', '\u{3009}'), // CJK angle brackets
        ('\u{FF08}', '\u{FF09}'), // fullwidth parentheses
    ] {
        assert_eq!(paired_bracket(open), Some((close, BracketKind::Open)));
        assert_eq!(paired_bracket(close), Some((open, BracketKind::Close)));
    }
    // BidiBrackets.txt header: "For legacy reasons, the characters
    // U+FD3E ORNATE LEFT PARENTHESIS and U+FD3F ORNATE RIGHT
    // PARENTHESIS do not mirror in bidirectional display and
    // therefore do not form a bracket pair."
    assert_eq!(paired_bracket('\u{FD3E}'), None);
    assert_eq!(paired_bracket('\u{FD3F}'), None);
    // Mirrored characters that are not Ps/Pe never pair.
    assert_eq!(paired_bracket('<'), None);
    assert_eq!(paired_bracket('\u{00AB}'), None);
}

/// BD16 close-branch: "Compare the closing paired bracket being
/// inspected to the bracket in the current stack element, where
/// U+3009 and U+232A are treated as equivalent." A U+2329 opener
/// therefore pairs with a U+3009 closer, and a U+3008 opener with a
/// U+232A closer.
#[test]
fn r283_bd16_canonical_equivalence_of_angle_brackets() {
    for (open, close) in [
        ('\u{2329}', '\u{3009}'),
        ('\u{3008}', '\u{232A}'),
        ('\u{2329}', '\u{232A}'),
        ('\u{3008}', '\u{3009}'),
    ] {
        let chars = ['a', open, 'b', close, 'c'];
        let classes: Vec<BidiClass> = chars.iter().copied().map(bidi_class).collect();
        assert_eq!(
            bracket_pairs(&chars, &classes),
            vec![(1, 3)],
            "U+{:04X} should pair with U+{:04X}",
            open as u32,
            close as u32
        );
    }
}

/// N0 end-to-end through the paragraph driver with a non-ASCII pair:
/// RTL paragraph, fullwidth parentheses around Latin text, an L
/// strong before the pair. Inside-strong L is opposite the (odd)
/// embedding direction and the preceding strong is also L, so N0 c.1
/// flips both brackets to L.
#[test]
fn r283_n0_fullwidth_parentheses_resolve_in_rtl_paragraph() {
    // Hebrew alef + bet, space, "a（b）" with fullwidth brackets.
    let text = "\u{05D0}\u{05D1} a\u{FF08}b\u{FF09}";
    let (bidi, _visual) = process_paragraph_with_brackets(text, None);
    assert_eq!(bidi.paragraph_level, 1);
    let chars: Vec<char> = text.chars().collect();
    let open_idx = chars.iter().position(|&c| c == '\u{FF08}').unwrap();
    let close_idx = chars.iter().position(|&c| c == '\u{FF09}').unwrap();
    // N0 c.1 resolves both brackets to L, so I2 gives them the even
    // level 2 (same as the enclosed Latin run), not the odd RTL
    // level the N1/N2 fallback would produce.
    assert_eq!(bidi.levels[open_idx], 2);
    assert_eq!(bidi.levels[close_idx], 2);
    assert_eq!(bidi.levels[open_idx + 1], 2); // 'b' inside
}

// --- Bidi_Mirroring_Glyph + rule L4 ---------------------------------

/// Non-bracket mirror pairs from BidiMirroring.txt: angle quotation
/// marks and mathematical relations.
#[test]
fn r283_mirrored_glyph_full_table_entries() {
    for (a, b) in [
        ('\u{00AB}', '\u{00BB}'), // double angle quotation marks
        ('\u{2039}', '\u{203A}'), // single angle quotation marks
        ('<', '>'),               // less-than / greater-than
        ('\u{2264}', '\u{2265}'), // less-than/greater-than or equal to
        ('\u{2282}', '\u{2283}'), // subset of / superset of
        ('\u{300A}', '\u{300B}'), // CJK double angle brackets
        ('\u{FE59}', '\u{FE5A}'), // small parentheses
    ] {
        assert_eq!(mirrored_glyph(a), Some(b));
        assert_eq!(mirrored_glyph(b), Some(a));
    }
    // Bidi_Mirrored=No characters have no entry.
    for c in ['a', '7', '\u{05D0}', '\u{FD3E}', '\u{FD3F}'] {
        assert_eq!(mirrored_glyph(c), None);
    }
}

/// L4: "A character is depicted by a mirrored glyph if and only if
/// (a) the resolved directionality of that character is R, and (b)
/// the Bidi_Mirrored property value of that character is Yes" — now
/// exercised with table entries beyond the ASCII brackets.
#[test]
fn r283_l4_mirrors_non_ascii_pairs_at_odd_levels() {
    let mut chars = ['\u{00AB}', '\u{05D0}', '\u{00BB}', '<', '\u{300A}'];
    apply_mirroring(&mut chars, &[1, 1, 1, 1, 1]);
    assert_eq!(chars, ['\u{00BB}', '\u{05D0}', '\u{00AB}', '>', '\u{300B}']);
    // Even levels (resolved directionality L) never mirror.
    let mut chars = ['\u{00AB}', 'x', '\u{2264}'];
    apply_mirroring(&mut chars, &[0, 0, 2]);
    assert_eq!(chars, ['\u{00AB}', 'x', '\u{2264}']);
}

/// L4 end-to-end: an RTL paragraph where a `≤` sits in the RTL run
/// (odd level → mirrored to `≥`) and the ornate parentheses stay
/// untouched (Bidi_Mirrored=No).
#[test]
fn r283_l4_end_to_end_rtl_paragraph() {
    let text = "\u{05D0}\u{2264}\u{05D1}\u{FD3E}";
    let (bidi, _visual) = process_paragraph_with_brackets(text, None);
    assert_eq!(bidi.paragraph_level, 1);
    let mut chars: Vec<char> = text.chars().collect();
    apply_mirroring(&mut chars, &bidi.levels);
    assert_eq!(chars[1], '\u{2265}', "≤ at odd level mirrors to ≥");
    assert_eq!(chars[3], '\u{FD3E}', "ornate parenthesis never mirrors");
}
