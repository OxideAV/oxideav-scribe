//! Round 175 — explicit script-tag entry point for caller-driven GSUB
//! feature application.
//!
//! [`oxideav_scribe::Face::shape_text_with_script`] takes an explicit
//! OpenType script tag and bypasses the script-tag priority walk
//! [`oxideav_scribe::Face::shape_text`] does. The new entry point is
//! useful in two scenarios:
//!
//! 1. The caller already knows the script of the run — typically
//!    because the run came out of a script-segmenter or because the
//!    caller is shaping a known-language string. Resolving against
//!    one explicit tag is deterministic and avoids the rare cross-
//!    script collision the auto-probe walk has when two scripts
//!    publish the same feature tag (e.g. `liga` under both `latn`
//!    and `arab`).
//! 2. The caller wants to test a non-Latin script's feature lookups
//!    that the auto-probe walk would never reach because an earlier
//!    script (e.g. `latn` / `DFLT`) already provides a matching
//!    feature tag.
//!
//! This test suite focuses on the contract — the entry point's
//! behaviour on known + unknown script tags, the agreement with the
//! auto-probe on shared resolutions, and the cmap-identity baseline
//! when the resolution yields no lookups. The deep "non-Latin script
//! actually reshapes" coverage is the auto-probe broadening test
//! cluster in `src/shaping/feature_subst.rs`.

use oxideav_scribe::Face;

const DEJAVU_BYTES: &[u8] = include_bytes!("../../oxideav-ttf/tests/fixtures/DejaVuSans.ttf");
const INTER_BYTES: &[u8] = include_bytes!("../tests/fixtures/InterVariable.ttf");

/// Explicit `latn` resolution matches the auto-probe output on
/// Inter's `smcp` — the auto-probe picks `latn` first for `smcp`, so
/// the two paths must produce identical glyph runs. This is the
/// no-regression baseline for the round-175 surface.
#[test]
fn explicit_latn_matches_auto_probe_smcp_on_inter() {
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let auto = face.shape_text("abc", &[*b"smcp"]);
    let explicit = face.shape_text_with_script("abc", *b"latn", &[*b"smcp"]);
    assert_eq!(auto, explicit);
}

/// Explicit `latn` resolution matches the auto-probe output on
/// Inter's `liga` — DejaVu publishes `liga` under `latn`, and the
/// auto-probe picks `latn` first. Same baseline as `smcp`, exercising
/// the LookupType-4 dispatch path.
#[test]
fn explicit_latn_matches_auto_probe_liga_on_dejavu() {
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    let auto = face.shape_text("fi", &[*b"liga"]);
    let explicit = face.shape_text_with_script("fi", *b"latn", &[*b"liga"]);
    assert_eq!(auto, explicit);
    assert_eq!(auto.len(), 1, "fi ligature collapses to one glyph");
}

/// Unknown script tag yields the cmap-identity output — every
/// requested feature resolves to an empty lookup list under an
/// unknown script. `zzzz` is not a registered OpenType script tag.
#[test]
fn unknown_script_tag_is_cmap_identity() {
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let cmap_only = face.shape_text("abc", &[]);
    let unknown = face.shape_text_with_script("abc", *b"zzzz", &[*b"smcp"]);
    assert_eq!(cmap_only, unknown);
}

/// Empty `text` always yields an empty run regardless of the script
/// tag or feature list — mirrors the [`Face::shape_text`] contract.
#[test]
fn empty_text_is_empty_vec() {
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let result = face.shape_text_with_script("", *b"latn", &[*b"smcp"]);
    assert!(result.is_empty());
}

/// Empty `features` slice always returns the pure-cmap output —
/// regardless of script-tag validity. The script tag is only
/// consulted when a feature needs resolving.
#[test]
fn empty_features_is_cmap_identity() {
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let cmap_only = face.shape_text("Hello", &[]);
    let explicit_empty = face.shape_text_with_script("Hello", *b"latn", &[]);
    let explicit_empty_unknown = face.shape_text_with_script("Hello", *b"zzzz", &[]);
    assert_eq!(cmap_only, explicit_empty);
    assert_eq!(cmap_only, explicit_empty_unknown);
}

/// `DFLT` resolution against an unknown feature tag is cmap identity.
/// `DFLT` is a registered OpenType script tag but won't publish a
/// feature tag the font hasn't declared.
#[test]
fn explicit_dflt_unknown_feature_is_cmap_identity() {
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let cmap_only = face.shape_text("abc", &[]);
    let explicit = face.shape_text_with_script("abc", *b"DFLT", &[*b"zzzz"]);
    assert_eq!(cmap_only, explicit);
}

/// Two requested features under the same explicit script tag are
/// applied in caller order. We use `smcp` (which reshapes lowercase)
/// followed by `case` (a no-op against lowercase) — the result
/// matches `smcp` alone, which mirrors the round-89 caller-order
/// test on the auto-probe surface.
#[test]
fn caller_order_preserved_under_explicit_script() {
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let smcp_alone = face.shape_text_with_script("abc", *b"latn", &[*b"smcp"]);
    let smcp_then_case = face.shape_text_with_script("abc", *b"latn", &[*b"smcp", *b"case"]);
    assert_eq!(smcp_alone, smcp_then_case);
}

/// Sanity: the cmap baseline produced by the explicit-script API
/// with an empty features list agrees with the auto-probe API. This
/// is the "the cmap fast-path is not different" guarantee.
#[test]
fn explicit_cmap_baseline_agrees_with_auto_probe() {
    let face = Face::from_ttf_bytes(INTER_BYTES.to_vec()).expect("Inter parses");
    let auto_cmap = face.shape_text("The quick brown fox", &[]);
    let explicit_cmap = face.shape_text_with_script("The quick brown fox", *b"latn", &[]);
    assert_eq!(auto_cmap, explicit_cmap);
    assert_eq!(auto_cmap.len(), "The quick brown fox".chars().count());
}
