//! Round 445 — the UAX #24 §5 itemisation refinements threaded through
//! the font-aware `Face` shaping surfaces.
//!
//! `script::script_runs` now resolves `Script_Extensions` constraint
//! sets (§5.3), inherits combining marks (§5.2), and pairs brackets
//! (§5.1). These tests verify the refinement is visible through the
//! public itemised-shaping pipeline — `Face::script_run_tags`,
//! `Face::shape_text_itemized`, `Face::position_text_itemized` — with
//! DejaVuSans (broad Latin/Greek/Cyrillic/Hebrew/Arabic coverage) as
//! the shaping face.

use intl::unicode::script::Script;
use oxideav_scribe::{resolve_scripts, Face};

const DEJAVU: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn face() -> Face {
    Face::from_ttf_bytes(DEJAVU.to_vec()).expect("DejaVuSans face")
}

#[test]
fn bracketed_greek_quotation_keeps_parens_on_the_latin_runs() {
    let face = face();
    let tags = face.script_run_tags("abc (\u{03A8}\u{03B1}) def");
    let seq: Vec<(Script, [u8; 4])> = tags.iter().map(|(r, t)| (r.script, *t)).collect();
    assert_eq!(
        seq,
        vec![
            (Script::Latin, *b"latn"),
            (Script::Greek, *b"grek"),
            (Script::Latin, *b"latn"),
        ],
        "got {tags:?}"
    );
    // The parentheses live in the Latin runs: Greek is exactly "Ψα".
    assert_eq!((tags[1].0.start, tags[1].0.end), (5, 7), "got {tags:?}");
}

#[test]
fn arabic_comma_joins_the_arabic_run_tag() {
    // U+060C ARABIC COMMA between Latin and Arabic text must shape
    // under the `arab` tag with the run it belongs to, not leak into
    // the `latn` run.
    let face = face();
    let tags = face.script_run_tags("ab\u{060C} \u{0628}\u{062A}");
    let seq: Vec<(Script, [u8; 4])> = tags.iter().map(|(r, t)| (r.script, *t)).collect();
    assert_eq!(
        seq,
        vec![(Script::Latin, *b"latn"), (Script::Arabic, *b"arab")],
        "got {tags:?}"
    );
    // The comma (index 2) is in the Arabic run.
    assert_eq!(tags[1].0.start, 2, "got {tags:?}");
}

#[test]
fn prolonged_sound_mark_is_not_swallowed_by_latin() {
    // U+30FC is scx = {Hira Kana}: after Latin it must open a Kana run
    // (resolving inside its scx set), not extend `latn`.
    let face = face();
    let tags = face.script_run_tags("abc\u{30FC}");
    assert_eq!(tags.len(), 2, "got {tags:?}");
    assert_eq!(tags[0].0.script, Script::Latin);
    assert_eq!(tags[1].1, *b"kana", "got {tags:?}");
}

#[test]
fn itemized_positioning_matches_per_run_explicit_script_shaping() {
    // The itemised path must equal the concatenation of the explicit
    // per-run shapings under the §5.1-refined run boundaries.
    let face = face();
    let text = "abc (\u{03A8}\u{03B1}) def";
    let whole = face.position_text_itemized(text, 16.0, &[]);
    let mut concat = Vec::new();
    for (run, tag) in face.script_run_tags(text) {
        let run_text: String = text
            .chars()
            .skip(run.start)
            .take(run.end - run.start)
            .collect();
        concat.extend(face.position_text_with_script(&run_text, 16.0, tag, &[]));
    }
    assert_eq!(whole.len(), concat.len());
    for (a, b) in whole.iter().zip(concat.iter()) {
        assert_eq!(a.glyph_id, b.glyph_id);
        assert_eq!(a.x_advance, b.x_advance);
    }
}

#[test]
fn itemized_gids_cover_every_char_of_a_bracketed_mixed_string() {
    // Non-ligating input: one glyph per char, none .notdef, through
    // the refined run partition.
    let face = face();
    let text = "a(\u{0431})b";
    let gids = face.shape_text_itemized(text, &[]);
    assert_eq!(gids.len(), text.chars().count(), "got {gids:?}");
    assert!(gids.iter().all(|&g| g != 0), "got {gids:?}");
}

#[test]
fn resolver_invariants_hold_on_hostile_input() {
    // Degenerate inputs: unpaired brackets, deep nesting past the
    // 63-entry stack, lone marks, pseudo-script soup. The resolver
    // must stay total (one output per input char) and never emit
    // Inherited/Unknown.
    let deep_open: String = "(".repeat(100);
    let deep_close: String = ")".repeat(100);
    let cases = [
        "",
        ")))(((",
        "\u{0301}\u{0301}\u{0301}",
        "\u{30FC}\u{060C}\u{0964}",
        "a\u{200D}\u{200C}\u{FE0F}b",
        &format!("a{deep_open}\u{03B1}{deep_close}z"),
        "\u{FFFF}\u{E000}\u{10FFFF}",
    ];
    for case in cases {
        let chars: Vec<char> = case.chars().collect();
        let resolved = resolve_scripts(&chars);
        assert_eq!(resolved.len(), chars.len(), "case {case:?}");
        for s in &resolved {
            assert!(
                !matches!(s, Script::Inherited | Script::Unknown),
                "case {case:?} resolved {s:?}"
            );
        }
    }
}
