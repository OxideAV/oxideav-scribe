//! Round 186 — UAX #9 BiDi character class lookup + paragraph-level
//! resolution (rules P1 / P2 / P3) integration tests.
//!
//! Mirrors the unit tests in `src/bidi.rs` but exercises the public
//! re-exports on `oxideav_scribe::` so the surface stays stable for
//! external callers. Provenance: every input here is constructed by
//! hand from UAX #9 Revision 50 / Unicode 16.0 (the dated snapshot
//! at `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`). No
//! library source was consulted.

use oxideav_scribe::{bidi_class, paragraph_level, split_paragraphs, BidiClass};

#[test]
fn r186_public_bidi_class_explicit_format_set() {
    // UAX #9 §2.1..§2.5 + §2.6: the 12 control-plane bidi-class
    // assignments. Asserted through the public `bidi_class` re-export
    // (not `oxideav_scribe::bidi::bidi_class`) so any future rename of
    // the inner symbol is caught here.
    assert_eq!(bidi_class('\u{200E}'), BidiClass::L); // LRM
    assert_eq!(bidi_class('\u{200F}'), BidiClass::R); // RLM
    assert_eq!(bidi_class('\u{061C}'), BidiClass::AL); // ALM
    assert_eq!(bidi_class('\u{202A}'), BidiClass::LRE);
    assert_eq!(bidi_class('\u{202B}'), BidiClass::RLE);
    assert_eq!(bidi_class('\u{202C}'), BidiClass::PDF);
    assert_eq!(bidi_class('\u{202D}'), BidiClass::LRO);
    assert_eq!(bidi_class('\u{202E}'), BidiClass::RLO);
    assert_eq!(bidi_class('\u{2066}'), BidiClass::LRI);
    assert_eq!(bidi_class('\u{2067}'), BidiClass::RLI);
    assert_eq!(bidi_class('\u{2068}'), BidiClass::FSI);
    assert_eq!(bidi_class('\u{2069}'), BidiClass::PDI);
}

#[test]
fn r186_public_paragraph_level_mixed_strings() {
    // Mixed-script real-world inputs.
    // Pure Latin → 0.
    assert_eq!(paragraph_level("Hello, world!"), 0);
    // Pure Hebrew "שלום" → 1.
    assert_eq!(paragraph_level("\u{05E9}\u{05DC}\u{05D5}\u{05DD}"), 1);
    // Pure Arabic "مرحبا" → 1.
    assert_eq!(
        paragraph_level("\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}"),
        1
    );
    // Mixed: leading Latin "X" decides → 0.
    assert_eq!(
        paragraph_level("X \u{0645}\u{0631}\u{062D}\u{0628}\u{0627}"),
        0
    );
    // Mixed: leading Hebrew decides → 1.
    assert_eq!(paragraph_level("\u{05D0} Hello"), 1);
    // Quoted English wrapped in an RLI ... PDI inside a Hebrew
    // paragraph: the RLI region is skipped by P2 — but the
    // surrounding text is Hebrew so first strong outside is Hebrew → 1.
    assert_eq!(
        paragraph_level("\u{05D0}\u{2067}English text\u{2069}\u{05D0}"),
        1
    );
    // English with a quoted Hebrew run isolated by LRI...PDI: the
    // bracketed region is skipped, English decides → 0.
    assert_eq!(
        paragraph_level("English \u{2066}\u{05D0}\u{05D1}\u{2069} more English"),
        0
    );
}

#[test]
fn r186_public_split_paragraphs_round_trips() {
    // P1 invariant: concatenating the returned paragraphs yields the
    // original text byte-for-byte.
    for s in [
        "",
        "no separators",
        "a\nb",
        "a\nb\n",
        "a\r\nb",
        "a\u{2029}b",
        "a\n\nb",
        "\u{05D0}\n\u{0627}",
    ] {
        let parts = split_paragraphs(s);
        let recon: String = parts.iter().copied().collect();
        assert_eq!(recon, s, "round-trip failed for {s:?}");
    }
}

#[test]
fn r186_public_paragraph_level_p1_then_p2_workflow() {
    // The typical UAX #9 entry sequence: P1 split, then run P2 + P3
    // per paragraph. Asserts paragraphs of different scripts inside
    // one input each resolve to their own level.
    let text = "First English paragraph.\n\u{05D0}\u{05D1}\u{05D2} Hebrew";
    let paras = split_paragraphs(text);
    assert_eq!(paras.len(), 2);
    assert_eq!(paragraph_level(paras[0]), 0); // Latin → LTR
    assert_eq!(paragraph_level(paras[1]), 1); // Hebrew → RTL
}

#[test]
fn r186_p3_default_when_no_strong_character() {
    // Per P3, when P2 finds no strong character (empty paragraph,
    // pure whitespace + digits + punctuation), the level defaults
    // to 0 (LTR).
    assert_eq!(paragraph_level("   "), 0);
    assert_eq!(paragraph_level("123 456"), 0);
    assert_eq!(paragraph_level("\u{2066}\u{05D0}"), 0); // unterminated LRI hides Hebrew → default 0
}

#[test]
fn r186_isolate_skip_handles_nested_pairs() {
    // Nested isolates: P2 must skip the entire outermost LRI ... PDI
    // region including a nested RLI ... PDI inside it.
    let s = "\u{2066}\u{2067}\u{0627}\u{2069}\u{2069}English";
    assert_eq!(paragraph_level(s), 0);
    // After the outermost PDI, the surrounding text decides. Replace
    // English with Hebrew to confirm.
    let s = "\u{2066}\u{2067}\u{0627}\u{2069}\u{2069}\u{05D0}";
    assert_eq!(paragraph_level(s), 1);
}
