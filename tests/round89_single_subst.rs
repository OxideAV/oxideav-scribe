//! Round 89 — GSUB LookupType 1 (Single Substitution) caller-driven
//! feature application via `Face::shape_text(text, features)`.
//!
//! ## What this exercises
//!
//! 1. `Face::shape_text(text, &[])` is cmap identity — no GSUB
//!    pass when the feature list is empty.
//! 2. Display-toggled features that round-15 (`ccmp` / `calt`)
//!    doesn't run by default: `smcp` (small caps), `sups`
//!    (superscript), `subs` (subscript), `case` (case-sensitive
//!    forms), `salt` (stylistic alternates), `ss01..ss20` (stylistic
//!    sets).
//! 3. Caller-controlled ordering: applying two features in sequence
//!    produces a different output than applying just one — both as
//!    a smoke test for the API surface and as a fixture against
//!    Inter Variable's documented feature set (covered by the
//!    round-88 snapshot in `round88_gsub_features.rs`).
//! 4. The round-89 surface is **LookupType 1 only**: pointing it
//!    at `liga` (Type 4) is a documented no-op so callers wanting
//!    ligature collapsing keep using `Shaper::shape` /
//!    `FaceChain::shape`.
//!
//! ## Worked shape example (Inter Variable, "Hi", `&[*b"smcp"]`)
//!
//! - cmap("H") = `41` (glyph index of the upper-case H in Inter's cmap)
//! - cmap("i") = `76`
//! - `smcp` Format 2 sub-table covers lowercase ASCII a..z; the "H"
//!   slot is outside coverage so passes through unchanged.
//! - "i" → small-cap variant glyph (Inter ships a dedicated small-cap
//!   ‹ɪ› at a different glyph id).
//!
//! Result: `[41, smcp_i]` — same length, first slot unchanged,
//! second slot reshaped. The `inter_smcp_only_reshapes_lowercase`
//! test below asserts exactly this shape.

use oxideav_scribe::Face;

const DEJAVU_BYTES: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
const INTER_BYTES: &[u8] = include_bytes!("fixtures/InterVariable.ttf");

fn cmap(face: &Face, ch: char) -> u16 {
    face.with_font(|font| font.glyph_index(ch).unwrap_or(0))
        .unwrap_or(0)
}

// ---------- empty / cmap-identity baselines -----------------------------

#[test]
fn empty_text_yields_empty_glyph_run() {
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    assert_eq!(face.shape_text("", &[]).len(), 0);
    assert_eq!(face.shape_text("", &[*b"smcp"]).len(), 0);
}

#[test]
fn empty_features_is_cmap_identity_on_pangram() {
    // No GSUB features = cmap-only output. This is the baseline every
    // subsequent feature test diffs against.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    let text = "The quick brown fox";
    let want: Vec<u16> = text.chars().map(|c| cmap(&face, c)).collect();
    assert_eq!(face.shape_text(text, &[]), want);
}

// ---------- Inter Variable: display-toggled features --------------------

#[test]
fn inter_smcp_only_reshapes_lowercase() {
    // smcp's Format 2 sub-table in Inter covers a..z but not A..Z, so
    // a mixed-case input must have its uppercase slot unchanged and
    // every lowercase slot reshaped.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let h_gid = cmap(&face, 'H');
    let i_gid = cmap(&face, 'i');

    let smcp = face.shape_text("Hi", &[*b"smcp"]);
    assert_eq!(smcp.len(), 2);
    assert_eq!(smcp[0], h_gid, "smcp doesn't cover uppercase H — unchanged");
    assert_ne!(
        smcp[1], i_gid,
        "smcp must reshape lowercase i to its small-cap variant"
    );
}

#[test]
fn inter_sups_substitutes_digit_slots() {
    // Inter ships dedicated superscript glyphs for every digit; we
    // assert the bulk of slots reshape, allowing one or two to stay
    // intact in case the font consolidates a digit with its
    // superscript (per the existing round-88 snapshot which doesn't
    // pin the per-glyph mapping).
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let cmap_digits: Vec<u16> = "0123456789".chars().map(|c| cmap(&face, c)).collect();
    let sups_digits = face.shape_text("0123456789", &[*b"sups"]);
    assert_eq!(sups_digits.len(), 10);
    let changed = cmap_digits
        .iter()
        .zip(sups_digits.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed >= 8,
        "sups must reshape most digits (got {changed}/10)"
    );
}

#[test]
fn inter_subs_distinct_from_sups() {
    // sups and subs are sibling features that target the same source
    // glyphs (digits) but emit different output glyphs (raised vs
    // lowered). The round-89 surface routes each independently.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let sups = face.shape_text("0123", &[*b"sups"]);
    let subs = face.shape_text("0123", &[*b"subs"]);
    assert_ne!(sups, subs);
}

#[test]
fn inter_salt_is_well_defined_on_lowercase() {
    // salt (stylistic alternates) typically reshapes a subset of
    // letters — exactly which depends on the font. For Inter the
    // documented coverage includes the lowercase 'a' (a single-storey
    // alternate). Verifying it's well-defined (returns a glyph run of
    // the right length and doesn't error) is enough; the specific
    // mapping is an Inter implementation detail we don't pin.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let salt = face.shape_text("abc", &[*b"salt"]);
    assert_eq!(salt.len(), 3);
}

// ---------- ordering --------------------------------------------------

#[test]
fn case_after_smcp_does_not_undo_smcp_on_lowercase() {
    // `case` (case-sensitive forms) targets bracket / paren / punctuation
    // glyphs — not letters. On pure-lowercase input, applying smcp then
    // case must produce the same glyph run as smcp alone (case is a
    // no-op for letters).
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let smcp = face.shape_text("abc", &[*b"smcp"]);
    let smcp_then_case = face.shape_text("abc", &[*b"smcp", *b"case"]);
    assert_eq!(smcp, smcp_then_case);
}

#[test]
fn applying_smcp_then_smcp_is_idempotent() {
    // smcp Format 2 maps a..z → small-cap glyphs. Re-applying smcp to
    // the small-cap output is a no-op because the new glyphs are not
    // in the smcp coverage table. This guards against a future
    // regression where coverage probing accidentally also matched the
    // post-substitution glyphs.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let once = face.shape_text("abc", &[*b"smcp"]);
    let twice = face.shape_text("abc", &[*b"smcp", *b"smcp"]);
    assert_eq!(once, twice, "smcp must be idempotent on its own output");
}

// ---------- single-substitution-only contract ---------------------------

#[test]
fn dejavu_liga_is_skipped_because_lookup_type_is_4() {
    // The round-89 brief explicitly scopes shape_text to LookupType 1.
    // DejaVu Sans's `liga` feature publishes a LookupType-4 lookup
    // (the fi / fl ligature). shape_text(&[liga]) must NOT collapse
    // "fi" — callers wanting ligatures go through Shaper::shape /
    // FaceChain::shape (which run the full multi-type pipeline).
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    assert!(face.has_gsub_feature(*b"latn", *b"liga"));
    let cmap_only = face.shape_text("fi", &[]);
    let liga_attempt = face.shape_text("fi", &[*b"liga"]);
    assert_eq!(
        cmap_only, liga_attempt,
        "liga (Type 4) is silently skipped by the round-89 Type-1-only surface"
    );
    assert_eq!(
        cmap_only.len(),
        2,
        "fi stays 2 glyphs without ligature collapse"
    );
}

#[test]
fn unknown_feature_tag_is_silent_noop() {
    // Requesting a feature tag the font doesn't publish must fall
    // through to cmap identity — no panic, no error.
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let baseline = face.shape_text("Hello", &[]);
    let bogus = face.shape_text("Hello", &[*b"zzzz"]);
    assert_eq!(baseline, bogus);
}

#[test]
fn font_without_feature_is_silent_noop() {
    // DejaVu Sans doesn't publish `smcp` (its small-caps are a
    // separate font file). shape_text(&[smcp]) on DejaVu must be
    // cmap identity.
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    assert!(!face.has_gsub_feature(*b"latn", *b"smcp"));
    let cmap_only = face.shape_text("abc", &[]);
    let smcp_attempt = face.shape_text("abc", &[*b"smcp"]);
    assert_eq!(cmap_only, smcp_attempt);
}
