//! Round 362 — positioned caller-feature shaping.
//!
//! Before this round the caller-driven GSUB feature surface
//! (`Face::shape_text` and friends) returned **bare glyph IDs**: a caller
//! that requested `smcp` / `frac` / `sups` / a stylistic set got the
//! substituted glyphs but no advances, kerning, mark attachment, or
//! contextual positioning. The always-on `Shaper::shape` pipeline ran
//! the full GPOS pass but only over the hard-coded `ccmp` / `liga` /
//! `calt` feature set, so the two halves never met.
//!
//! Round 362 bridges them: `Face::position_text` (single-face),
//! `FaceChain::shape_with_features` (multi-face fallback), and the
//! lower-level `position_*` entry points in `shaping::feature_subst` run
//! the caller's requested GSUB features and then feed the substituted run
//! through the same kerning → SinglePos → mark-to-base / mark-to-mark /
//! mark-to-ligature → cursive → contextual-positioning sequence the
//! always-on pipeline uses.
//!
//! Wall: spec basis is the project-vendored OpenType OFF GPOS chapter
//! (positioning-pass ordering) and the registered-feature catalogue under
//! `docs/text/opentype/registries/`. No external library, no web.

use oxideav_scribe::{Face, FaceChain};

const INTER: &[u8] = include_bytes!("fixtures/InterVariable.ttf");
const DEJAVU: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

/// `Face::position_text` carries the same glyph IDs as the GID-only
/// `Face::shape_text` path — positioning never changes which glyph is in
/// each slot, only its advance / offset.
#[test]
fn position_text_gids_match_shape_text() {
    let face = Face::from_ttf_bytes(INTER.to_vec()).expect("Inter parses");
    let gids = face.shape_text("abc", &[*b"smcp"]);
    let placed = face.position_text("abc", 32.0, &[*b"smcp"]);
    let placed_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    assert_eq!(
        gids, placed_gids,
        "small-cap GIDs must match the subst path"
    );
    assert_eq!(placed.len(), 3);
    assert!(
        placed.iter().all(|g| g.x_advance > 0.0),
        "small caps are not zero-width"
    );
}

/// Empty features positions the pure-cmap run; advances come from hmtx.
#[test]
fn position_text_empty_features_is_cmap_positioned() {
    let face = Face::from_ttf_bytes(DEJAVU.to_vec()).expect("DejaVu parses");
    let placed = face.position_text("abc", 24.0, &[]);
    assert_eq!(placed.len(), 3);
    assert!(placed.iter().all(|g| g.x_advance > 0.0));
}

/// A `liga`-collapsed "fi" run is positioned as a single glyph whose
/// advance equals the ligature glyph's own scaled hmtx advance.
#[test]
fn position_text_liga_advances_as_one_glyph() {
    let face = Face::from_ttf_bytes(DEJAVU.to_vec()).expect("DejaVu parses");
    let placed = face.position_text("fi", 40.0, &[*b"liga"]);
    assert_eq!(placed.len(), 1, "fi collapses to one ligature glyph");
    let lig_gid = placed[0].glyph_id;
    let want = face
        .with_font(|font| {
            let scale = 40.0 / font.units_per_em().max(1) as f32;
            font.glyph_advance(lig_gid) as f32 * scale
        })
        .unwrap();
    assert!(
        (placed[0].x_advance - want).abs() < 1e-3,
        "ligature advance {} must equal scaled hmtx {want}",
        placed[0].x_advance
    );
}

/// Degenerate inputs return an empty positioned run.
#[test]
fn position_text_degenerate_inputs_are_empty() {
    let face = Face::from_ttf_bytes(DEJAVU.to_vec()).expect("DejaVu parses");
    assert!(face.position_text("", 16.0, &[]).is_empty());
    assert!(face.position_text("abc", 0.0, &[]).is_empty());
    assert!(face.position_text("abc", -1.0, &[]).is_empty());
}

/// The explicit-script-tag positioned entry resolves the feature against
/// the given script and matches the GID-only script-explicit path.
#[test]
fn position_text_with_script_matches_subst() {
    let face = Face::from_ttf_bytes(INTER.to_vec()).expect("Inter parses");
    let gids = face.shape_text_with_script("abc", *b"latn", &[*b"smcp"]);
    let placed = face.position_text_with_script("abc", 18.0, *b"latn", &[*b"smcp"]);
    let placed_gids: Vec<u16> = placed.iter().map(|g| g.glyph_id).collect();
    assert_eq!(gids, placed_gids);
}

/// `FaceChain::shape_with_features` with an empty feature list produces
/// the same per-glyph advances as the chain's always-on `shape` for a
/// plain Latin run (no GSUB feature differences, identical GPOS).
#[test]
fn facechain_empty_features_advances_match_shape() {
    let face = Face::from_ttf_bytes(DEJAVU.to_vec()).expect("DejaVu parses");
    let chain = FaceChain::new(face);
    // The always-on path runs ccmp/liga/calt; the empty-feature path runs
    // none of them. For a run with no ligatures/ccmp ("xyz" has no
    // DejaVu liga or ccmp coverage), the glyph runs and advances match.
    let always = chain.shape("xyz", 20.0).expect("shape ok");
    let feat = chain
        .shape_with_features("xyz", 20.0, &[])
        .expect("shape_with_features ok");
    assert_eq!(always.len(), feat.len());
    for (a, f) in always.iter().zip(feat.iter()) {
        assert_eq!(a.glyph_id, f.glyph_id);
        assert!((a.x_advance - f.x_advance).abs() < 1e-3);
    }
}

/// `FaceChain::shape_with_features` applies the requested feature: small
/// caps on Inter reshapes the run vs. the empty-feature baseline.
#[test]
fn facechain_smcp_reshapes_run() {
    let face = Face::from_ttf_bytes(INTER.to_vec()).expect("Inter parses");
    let chain = FaceChain::new(face);
    let base = chain
        .shape_with_features("abc", 24.0, &[])
        .expect("baseline ok");
    let smcp = chain
        .shape_with_features("abc", 24.0, &[*b"smcp"])
        .expect("smcp ok");
    assert_eq!(base.len(), smcp.len());
    let base_gids: Vec<u16> = base.iter().map(|g| g.glyph_id).collect();
    let smcp_gids: Vec<u16> = smcp.iter().map(|g| g.glyph_id).collect();
    assert_ne!(
        base_gids, smcp_gids,
        "smcp must reshape lowercase ASCII to small caps"
    );
    assert!(smcp.iter().all(|g| g.x_advance > 0.0));
}

/// Empty text / non-positive size return an empty run from the chain
/// surface too.
#[test]
fn facechain_degenerate_inputs_are_empty() {
    let face = Face::from_ttf_bytes(DEJAVU.to_vec()).expect("DejaVu parses");
    let chain = FaceChain::new(face);
    assert!(chain.shape_with_features("", 16.0, &[]).unwrap().is_empty());
    assert!(chain
        .shape_with_features("abc", 0.0, &[])
        .unwrap()
        .is_empty());
}
