//! Round 377 — `Face::position_text_itemized`: segmenter-driven
//! per-script shaping over a real font (DejaVuSans, which carries Latin,
//! Cyrillic, Greek, Hebrew and Arabic coverage plus GSUB/GPOS).

use intl::unicode::script::Script;
use oxideav_scribe::{ot_script_tag, script_runs_str, Face};

const DEJAVU_BYTES: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn dejavu() -> Face {
    Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses")
}

#[test]
fn resolve_tag_uses_registered_script() {
    // DejaVuSans registers latn / cyrl / grek / hebr / arab in its GSUB
    // ScriptList; resolution returns those real tags.
    let face = dejavu();
    assert_eq!(face.resolve_ot_script_tag(Script::Latin), *b"latn");
    assert_eq!(face.resolve_ot_script_tag(Script::Cyrillic), *b"cyrl");
    assert_eq!(face.resolve_ot_script_tag(Script::Greek), *b"grek");
    assert_eq!(face.resolve_ot_script_tag(Script::Hebrew), *b"hebr");
    assert_eq!(face.resolve_ot_script_tag(Script::Arabic), *b"arab");
}

#[test]
fn resolve_tag_falls_back_to_primary_for_unregistered_script() {
    // DejaVuSans does NOT register Devanagari; resolution falls back to
    // the primary (modern v.2) registry tag rather than failing.
    let face = dejavu();
    assert_eq!(
        face.resolve_ot_script_tag(Script::Devanagari),
        ot_script_tag(Script::Devanagari)
    );
    assert_eq!(face.resolve_ot_script_tag(Script::Devanagari), *b"dev2");
}

#[test]
fn empty_and_degenerate_inputs() {
    let face = dejavu();
    assert!(face.position_text_itemized("", 16.0, &[]).is_empty());
    assert!(face.position_text_itemized("abc", 0.0, &[]).is_empty());
    assert!(face.position_text_itemized("abc", f32::NAN, &[]).is_empty());
}

#[test]
fn pure_latin_itemized_equals_single_run() {
    // For a single-script string, itemised shaping must produce exactly
    // the same glyphs as shaping the whole thing under the resolved tag.
    let face = dejavu();
    let text = "Hello, world!";
    let itemized = face.position_text_itemized(text, 16.0, &[]);
    let single = face.position_text_with_script(text, 16.0, *b"latn", &[]);
    assert_eq!(itemized, single, "single-script itemised path must match");
    assert!(!itemized.is_empty());
}

#[test]
fn glyph_count_equals_sum_of_runs() {
    // The concatenated output length equals the sum of the per-run
    // lengths, each run positioned under its own tag.
    let face = dejavu();
    // Latin + space + Cyrillic.
    let text = "abc \u{0414}\u{0430}";
    let runs = script_runs_str(text);
    assert_eq!(runs.len(), 2, "{runs:?}");

    let mut expected = 0usize;
    let chars: Vec<char> = text.chars().collect();
    for run in &runs {
        let run_text: String = chars[run.start..run.end].iter().collect();
        let tag = ot_script_tag(run.script);
        expected += face
            .position_text_with_script(&run_text, 16.0, tag, &[])
            .len();
    }

    let itemized = face.position_text_itemized(text, 16.0, &[]);
    assert_eq!(itemized.len(), expected);
    assert!(itemized.len() >= 5, "all glyphs accounted for");
}

#[test]
fn mixed_run_uses_distinct_tags() {
    // Confirm the itemiser actually selected two different OT tags for a
    // Latin + Cyrillic string (regression guard on the segmenter wiring).
    let runs = script_runs_str("abc\u{0414}");
    let tags: Vec<[u8; 4]> = runs.iter().map(|r| ot_script_tag(r.script)).collect();
    assert_eq!(tags, vec![*b"latn", *b"cyrl"], "{runs:?}");
}

#[test]
fn no_glyph_is_notdef_for_covered_scripts() {
    // DejaVuSans covers Latin + Cyrillic + Greek; an itemised run over
    // those scripts should map every character to a real glyph, never
    // the .notdef tofu (gid 0).
    let face = dejavu();
    let text = "ab\u{0414}\u{0435}\u{03B1}\u{03B2}"; // Latin, Cyrillic, Greek
    let placed = face.position_text_itemized(text, 16.0, &[]);
    assert!(!placed.is_empty());
    for g in &placed {
        assert_ne!(
            g.glyph_id, 0,
            "unexpected .notdef in covered run: {placed:?}"
        );
    }
}

#[test]
fn shape_text_itemized_matches_single_for_pure_latin() {
    let face = dejavu();
    let text = "Hello";
    let it = face.shape_text_itemized(text, &[]);
    let single = face.shape_text_with_script(text, *b"latn", &[]);
    assert_eq!(it, single);
    assert!(!it.is_empty());
    assert!(it.iter().all(|&g| g != 0));
}

#[test]
fn shape_text_itemized_empty() {
    let face = dejavu();
    assert!(face.shape_text_itemized("", &[]).is_empty());
}

#[test]
fn shape_text_itemized_mixed_no_notdef() {
    // Latin + Cyrillic + Greek, all covered by DejaVu.
    let face = dejavu();
    let text = "ab\u{0414}\u{0435}\u{03B1}\u{03B2}";
    let gids = face.shape_text_itemized(text, &[]);
    assert!(!gids.is_empty());
    assert!(gids.iter().all(|&g| g != 0), "{gids:?}");
}

#[test]
fn script_run_tags_partition_and_tags() {
    let face = dejavu();
    // Latin, space (Common -> Latin), Cyrillic.
    let text = "abc \u{0414}\u{0430}";
    let tagged = face.script_run_tags(text);
    assert_eq!(tagged.len(), 2, "{tagged:?}");
    assert_eq!(tagged[0].1, *b"latn");
    assert_eq!(tagged[1].1, *b"cyrl");
    // Total partition over char indices.
    let n = text.chars().count();
    assert_eq!(tagged[0].0.start, 0);
    assert_eq!(tagged.last().unwrap().0.end, n);
    assert_eq!(tagged[0].0.end, tagged[1].0.start);
}
