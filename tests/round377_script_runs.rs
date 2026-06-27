//! Round 377 — script itemisation: Unicode-script → OpenType-tag map
//! and script-run segmentation through the crate's public API.

use intl::unicode::script::Script;
use oxideav_scribe::{ot_script_tag, ot_script_tags, script_runs_str, ScriptRun};

#[test]
fn ot_tag_core_repertoire() {
    assert_eq!(ot_script_tag(Script::Latin), *b"latn");
    assert_eq!(ot_script_tag(Script::Cyrillic), *b"cyrl");
    assert_eq!(ot_script_tag(Script::Greek), *b"grek");
    assert_eq!(ot_script_tag(Script::Arabic), *b"arab");
    assert_eq!(ot_script_tag(Script::Hebrew), *b"hebr");
    assert_eq!(ot_script_tag(Script::Han), *b"hani");
    assert_eq!(ot_script_tag(Script::Hangul), *b"hang");
    assert_eq!(ot_script_tag(Script::Thai), *b"thai");
}

#[test]
fn ot_tag_indic_dual_pairs_modern_first() {
    for (s, modern, legacy) in [
        (Script::Bengali, *b"bng2", *b"beng"),
        (Script::Devanagari, *b"dev2", *b"deva"),
        (Script::Gujarati, *b"gjr2", *b"gujr"),
        (Script::Gurmukhi, *b"gur2", *b"guru"),
        (Script::Kannada, *b"knd2", *b"knda"),
        (Script::Malayalam, *b"mlm2", *b"mlym"),
        (Script::Oriya, *b"ory2", *b"orya"),
        (Script::Tamil, *b"tml2", *b"taml"),
        (Script::Telugu, *b"tel2", *b"telu"),
        (Script::Myanmar, *b"mym2", *b"mymr"),
    ] {
        assert_eq!(ot_script_tags(s), &[modern, legacy], "{s:?}");
        assert_eq!(ot_script_tag(s), modern, "{s:?} primary");
    }
}

#[test]
fn every_real_run_has_a_non_default_tag() {
    // For the scripts we segment in real text below, the tag must not be
    // DFLT — DFLT is reserved for the script-less pseudo-scripts.
    for s in [
        Script::Latin,
        Script::Cyrillic,
        Script::Hebrew,
        Script::Arabic,
        Script::Han,
    ] {
        assert_ne!(ot_script_tag(s), *b"DFLT", "{s:?} should be a real tag");
    }
}

fn scripts(runs: &[ScriptRun]) -> Vec<Script> {
    runs.iter().map(|r| r.script).collect()
}

#[test]
fn english_hebrew_arabic_mixed_paragraph() {
    // "Hello שלום مرحبا" — Latin, space (Common→Latin), Hebrew, space
    // (Common→Hebrew), Arabic.
    let runs = script_runs_str("Hello \u{05E9}\u{05DC}\u{05D5}\u{05DD} \u{0645}\u{0631}\u{062D}");
    assert_eq!(
        scripts(&runs),
        vec![Script::Latin, Script::Hebrew, Script::Arabic],
        "{runs:?}"
    );
    // Total partition: first run starts at 0, last ends at char count.
    let n = "Hello \u{05E9}\u{05DC}\u{05D5}\u{05DD} \u{0645}\u{0631}\u{062D}"
        .chars()
        .count();
    assert_eq!(runs[0].start, 0);
    assert_eq!(runs.last().unwrap().end, n);
}

#[test]
fn cjk_and_latin_split() {
    // "ABC世界123" — Latin "ABC", Han "世界", then digits (Common) that
    // back-fill is not triggered (they follow Han, so they join Han).
    let runs = script_runs_str("ABC\u{4E16}\u{754C}123");
    assert_eq!(scripts(&runs), vec![Script::Latin, Script::Han], "{runs:?}");
    // The trailing digits joined the Han run.
    assert_eq!(runs[1].end, "ABC\u{4E16}\u{754C}123".chars().count());
}

#[test]
fn pure_number_string_is_one_common_run() {
    // No real script ever appears: the whole thing is a single Common
    // run mapping to DFLT.
    let runs = script_runs_str("12.34 + 56");
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].script, Script::Common);
    assert_eq!(ot_script_tag(runs[0].script), *b"DFLT");
}

#[test]
fn combining_marks_never_open_a_run() {
    // "café" with a combining acute on the e: "cafe\u{0301}" — all Latin.
    let runs = script_runs_str("cafe\u{0301}");
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].script, Script::Latin);
}
