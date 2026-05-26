//! Round 156 — GSUB LookupType 3 (Alternate Substitution) caller-driven
//! feature application via `Face::shape_text(text, features)`.
//!
//! ## What this exercises
//!
//! 1. `aalt` (Access All Alternates) — every test fixture font in
//!    `tests/fixtures/` publishes `aalt` under `latn` / `DFLT`. The
//!    feature mixes Type-1 (Single) and Type-3 (Alternate) lookups;
//!    pre-round-156 the Type-3 component was silently skipped, so a
//!    glyph covered *only* by the Type-3 lookup passed through
//!    unchanged. The round-156 dispatch wires
//!    `gsub_apply_lookup_type_3(_, gid, 0)` into the caller-driven
//!    surface, so those slots now reshape to their default-alternate
//!    glyph.
//! 2. Length preservation — Type 3 is length-preserving (one alternate
//!    per covered slot), so `len(shape_text(text, &[*b"aalt"]))` ==
//!    `len(cmap_only)` for every input.
//! 3. Coverage gating — applying `aalt` to a string whose glyphs are
//!    outside the Type-3 lookup's coverage falls through to the
//!    Type-1 component (if any) or pure cmap identity.
//! 4. Default-alternate-index contract — the spec doesn't pin which
//!    alternate the layout engine picks. Round 156 defaults to
//!    `alternateIndex = 0` (the first entry in each `AlternateSet`);
//!    a higher-level surface that wanted user-driven indices would
//!    have to live above this layer.
//! 5. Re-applying `aalt` must be idempotent — the AlternateSet
//!    coverage is on the *input* glyphs, not the substitutes, so a
//!    second pass is a no-op.
//!
//! ## Worked example (Inter Variable, lookup 1 = Type 3 inside `aalt`)
//!
//! Inter's `aalt` references lookups `[0 (Type 1), 1 (Type 3)]`. The
//! Type-3 lookup covers `A`, `a`, `b`, `c`, `d`, `e`, `f`, `g`, and
//! more — for each, `AlternateSet[0]` is a stylistic alternate the
//! font's designer marked as the default-alternate variant. Pre-156,
//! `shape_text("a", &[*b"aalt"])` returned `cmap('a')`. Post-156 it
//! returns `Inter.gsub_apply_lookup_type_3(1, cmap('a'), 0) = 1562`.

use oxideav_scribe::Face;

const DEJAVU_BYTES: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
const INTER_BYTES: &[u8] = include_bytes!("fixtures/InterVariable.ttf");

fn cmap(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

// ---------- Inter Variable: aalt mixes Type 1 + Type 3 -------------------

#[test]
fn inter_aalt_substitutes_via_lookup_type_3() {
    // Inter's `aalt` references lookup indices [0 (Type 1), 1 (Type 3)].
    // Lookup 1 covers lowercase 'a', and its AlternateSet[0] differs
    // from cmap('a'). Post-round-156 the slot reshapes; pre-156 it
    // stayed at cmap('a') because Type 3 was silently skipped.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    assert!(face.has_gsub_feature(*b"latn", *b"aalt"));
    let cmap_a = cmap(&face, 'a');
    let aalt = face.shape_text("a", &[*b"aalt"]);
    assert_eq!(aalt.len(), 1, "Type 3 is length-preserving");
    assert_ne!(
        aalt[0], cmap_a,
        "round-156 reshapes 'a' via Type-3 alternate-0"
    );
}

#[test]
fn inter_aalt_reshapes_many_lowercase_slots() {
    // Inter's `aalt` Type-3 lookup covers a generous slice of the
    // lowercase ASCII range. We probe the b..g letters because the
    // round-156 probe in scribe's dev notes shows them as hits.
    // We don't pin the per-character mapping — just that the bulk of
    // slots reshape, so the round-156 dispatch is firing on multiple
    // coverage entries rather than a one-off lucky match.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let probe = "abcdefg";
    let cmap_only = face.shape_text(probe, &[]);
    let aalt = face.shape_text(probe, &[*b"aalt"]);
    assert_eq!(cmap_only.len(), probe.len());
    assert_eq!(aalt.len(), probe.len(), "Type 3 is length-preserving");
    let changed = cmap_only
        .iter()
        .zip(aalt.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed >= 5,
        "aalt must reshape most lowercase letters via Type-3 (got {changed}/{})",
        probe.len()
    );
}

// ---------- DejaVu Sans: aalt is entirely Type 3 -------------------------

#[test]
fn dejavu_aalt_is_pure_type_3() {
    // DejaVu's `aalt` references a single Type-3 lookup (index 30).
    // Coverage is small (5 hits across ASCII probes) but specific: 'I',
    // 'J', 'a', 'l', 'y'. Pre-round-156, applying aalt was a no-op on
    // DejaVu because the Type-3 lookup was silently skipped. Post-156
    // it reshapes those five slots and leaves everything else alone.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    assert!(face.has_gsub_feature(*b"latn", *b"aalt"));
    let probe = "Iaaly";
    let cmap_only = face.shape_text(probe, &[]);
    let aalt = face.shape_text(probe, &[*b"aalt"]);
    assert_eq!(cmap_only.len(), probe.len());
    assert_eq!(aalt.len(), probe.len(), "Type 3 is length-preserving");
    assert_ne!(
        cmap_only, aalt,
        "DejaVu's aalt must reshape at least one slot via Type 3"
    );
}

#[test]
fn dejavu_aalt_outside_coverage_is_cmap_identity() {
    // The Type-3 lookup's coverage in DejaVu is sparse. Letters that
    // aren't in the AlternateSet ('b', 'c', 'd', 'e', 'f', 'g' on
    // DejaVu — none of them are aalt-covered) pass through unchanged.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    let probe = "bcdefg"; // sub-string DejaVu's aalt doesn't cover
    let cmap_only = face.shape_text(probe, &[]);
    let aalt = face.shape_text(probe, &[*b"aalt"]);
    assert_eq!(
        cmap_only, aalt,
        "aalt must be cmap-identity on uncovered slots"
    );
}

// ---------- length-preservation + idempotence contracts -----------------

#[test]
fn aalt_is_idempotent_on_inter() {
    // Re-applying aalt is a no-op because the AlternateSet coverage
    // is on the *input* glyphs (typically the canonical alphabet
    // codepoints), not on the substitute glyphs. This guards against
    // a future regression where Type-3 coverage probing accidentally
    // matched the post-substitution alternate glyphs.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let once = face.shape_text("abcdefg", &[*b"aalt"]);
    let twice = face.shape_text("abcdefg", &[*b"aalt", *b"aalt"]);
    assert_eq!(
        once, twice,
        "aalt's Type-3 component must be idempotent on its own output"
    );
}

#[test]
fn aalt_does_not_affect_run_length() {
    // Type 3 is length-preserving — invariant against the input
    // composition. Applies to both Inter (rich coverage) and DejaVu
    // (sparse coverage).
    for bytes in [INTER_BYTES, DEJAVU_BYTES] {
        let face = Face::from_ttf_bytes(bytes.to_vec()).expect("font parses");
        for sample in ["a", "ab", "Hello, world!", "abcdefghijklmnop"] {
            let cmap_only = face.shape_text(sample, &[]);
            let aalt = face.shape_text(sample, &[*b"aalt"]);
            assert_eq!(
                cmap_only.len(),
                aalt.len(),
                "aalt must be length-preserving on {sample:?}"
            );
        }
    }
}

// ---------- font-without-aalt control ----------------------------------

#[test]
fn font_without_aalt_is_cmap_identity() {
    // We don't have a fixture without `aalt`, but we can construct a
    // pseudo-control by asking for a Type-3-only-routing feature tag
    // the fixtures don't ship: `zzzz` is not a registered OpenType
    // feature so the dispatch returns immediately with no lookups
    // resolved. This is the same control as the round-89 surface and
    // protects the round-156 wiring against a regression where an
    // unknown feature accidentally pulled in a Type-3 lookup.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let cmap_only = face.shape_text("abcdefg", &[]);
    let bogus = face.shape_text("abcdefg", &[*b"zzzz"]);
    assert_eq!(cmap_only, bogus);
}
