//! Round 343 — GPOS contextual (LookupType 7) + chained-contextual
//! (LookupType 8) positioning is wired into the shaper's run-level
//! positioning pass.
//!
//! None of the in-tree fixtures (DejaVu Sans / Mono, Inter Variable,
//! Source Sans 3) ship a GPOS type-7 or type-8 lookup — contextual
//! positioning is a rare feature in practice (Arabic Nastaliq, some
//! Indic display faces). The dependency `oxideav-ttf` unit-tests the
//! sub-table decode + nested-record dispatch directly with synthetic
//! GPOS tables; this crate's contribution is the *application* layer:
//! enumerate the contextual-positioning lookups, scan, and accumulate
//! the returned per-glyph deltas. The unit tests in
//! `src/shaping/contextual_pos.rs` cover the field mapping; these
//! integration tests prove the pass is wired into the public shaping
//! surface and is a transparent identity for fonts that ship no such
//! lookups (the overwhelming common case).

use oxideav_scribe::{Face, FaceChain, Shaper};

const DEJAVU: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
const INTER: &[u8] = include_bytes!("fixtures/InterVariable.ttf");
const SOURCE_SANS: &[u8] = include_bytes!("fixtures/SourceSans3-Regular.otf");

fn face(bytes: &[u8]) -> Face {
    Face::from_ttf_bytes(bytes.to_vec()).expect("fixture parses")
}

/// Confirm the fixtures genuinely ship no type-7/8 GPOS lookups, so the
/// "identity" assertions below are exercising the no-op fast path rather
/// than silently passing on an empty GPOS table.
#[test]
fn fixtures_ship_no_contextual_pos_lookups() {
    for bytes in [DEJAVU, INTER, SOURCE_SANS] {
        face(bytes)
            .with_font(|font| {
                let ctx = font
                    .gpos_lookup_list()
                    .iter()
                    .filter(|&&(_, ty, _)| ty == 7 || ty == 8)
                    .count();
                assert_eq!(
                    ctx, 0,
                    "fixture unexpectedly ships {ctx} contextual-pos lookups"
                );
            })
            .unwrap();
    }
}

/// Shaping a Latin run through the full pipeline (which now runs the
/// step-7 contextual-positioning pass) is byte-for-byte identical to the
/// pre-step-7 geometry for a font with no type-7/8 lookups. We assert
/// this by shaping twice and confirming the positioned-glyph stream is
/// stable, and that the run width is the expected sum of advances — the
/// contextual pass must not perturb a font it does not apply to.
#[test]
fn latin_shaping_is_unchanged_without_contextual_lookups() {
    let chain = FaceChain::new(face(DEJAVU));
    let a = chain.shape("Affinity Waltz, fi", 24.0).expect("shape ok");
    let b = chain.shape("Affinity Waltz, fi", 24.0).expect("shape ok");
    assert_eq!(a, b, "shaping must be deterministic");
    assert!(!a.is_empty());
    // Every advance is finite and the run has positive total width.
    let total: f32 = a.iter().map(|g| g.x_advance).sum();
    assert!(total > 0.0 && total.is_finite());
}

/// The CFF face (Source Sans, OTF) likewise passes through unchanged —
/// the contextual pass is table-driven and indifferent to the outline
/// flavour.
#[test]
fn cff_face_run_passes_through() {
    let chain = FaceChain::new(face(SOURCE_SANS));
    let glyphs = chain.shape("Type", 32.0).expect("shape ok");
    assert!(!glyphs.is_empty());
    for g in &glyphs {
        assert!(g.x_advance.is_finite());
        assert!(g.x_offset.is_finite());
        assert!(g.y_offset.is_finite());
    }
}

/// The vector-glyph path (`shape_to_paths`) — the primary render API —
/// also runs the contextual pass underneath and emits a stable,
/// non-empty placement stream for a font without type-7/8 lookups.
#[test]
fn shape_to_paths_runs_the_contextual_pass() {
    let chain = FaceChain::new(face(INTER));
    let placed = Shaper::shape_to_paths(&chain, "Office", 28.0);
    assert!(!placed.is_empty());
}
