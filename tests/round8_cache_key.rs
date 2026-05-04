//! Round-8 / #357 vector-text test: `Shaper::shape_to_paths` wraps
//! each glyph in a `Group { cache_key: Some(_), .. }` whose key is
//! deterministic per `(face_stable_id, glyph_id, size_q8)`.
//!
//! Acceptance:
//! - The same `(face, glyph, size)` tuple always produces the same
//!   `cache_key` (cache hit across calls).
//! - Different glyphs produce different keys.
//! - Different sizes produce different keys.
//! - A separately-loaded `Face` parsed from the same bytes produces
//!   the SAME key (the producer-side identity is content-derived, so
//!   the downstream rasterizer's bitmap cache survives renderer
//!   restarts).

use oxideav_core::Node;
use oxideav_scribe::{Face, FaceChain, Shaper};

const FIXTURE: &[u8] = include_bytes!("fixtures/DejaVuSansMono.ttf");

/// Pull the per-glyph cache_keys out of `shape_to_paths`'s output. Asserts
/// every emitted node is a `Group` with `cache_key = Some(_)` (round 8
/// guarantee).
fn cache_keys(face: Face, text: &str, size_px: f32) -> Vec<u64> {
    let chain = FaceChain::new(face);
    let placed = Shaper::shape_to_paths(&chain, text, size_px);
    placed
        .into_iter()
        .map(|(_face_idx, node, _t)| match node {
            Node::Group(g) => g
                .cache_key
                .expect("shape_to_paths must wrap each glyph in a cache-keyed Group"),
            other => panic!("expected Node::Group wrapper, got {other:?}"),
        })
        .collect()
}

#[test]
fn same_glyph_same_size_produces_same_cache_key() {
    // Two independent loads of the same font bytes — `stable_id`
    // should match, so the per-glyph cache_keys must match too.
    let face_a = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("parses");
    let face_b = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("parses");
    assert_ne!(
        face_a.id(),
        face_b.id(),
        "Face::id is per-process and must be unique per Face instance",
    );
    assert_eq!(
        face_a.stable_id(),
        face_b.stable_id(),
        "stable_id must be deterministic across loads of the same bytes",
    );

    let keys_a = cache_keys(face_a, "Hi", 32.0);
    let keys_b = cache_keys(face_b, "Hi", 32.0);
    assert_eq!(keys_a.len(), 2, "expected 2 glyphs for 'Hi'");
    assert_eq!(
        keys_a, keys_b,
        "two loads of the same font should yield identical glyph cache keys: {keys_a:?} vs {keys_b:?}",
    );
}

#[test]
fn different_glyphs_produce_different_cache_keys() {
    // 'H' and 'i' are obviously distinct glyphs in DejaVu Sans Mono,
    // and shape_to_paths preserves their order.
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("parses");
    let keys = cache_keys(face, "Hi", 32.0);
    assert_eq!(keys.len(), 2);
    assert_ne!(
        keys[0], keys[1],
        "distinct glyphs ('H' vs 'i') must hash to different cache keys: {keys:?}",
    );
}

#[test]
fn different_sizes_produce_different_cache_keys() {
    // Same glyph, different sizes — the size_q8 input must change the
    // hash so the rasterizer doesn't reuse a 16-px bitmap for a 32-px
    // request.
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("parses");
    let keys_16 = cache_keys(face, "H", 16.0);
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("parses");
    let keys_32 = cache_keys(face, "H", 32.0);
    assert_eq!(keys_16.len(), 1);
    assert_eq!(keys_32.len(), 1);
    assert_ne!(
        keys_16[0], keys_32[0],
        "size_px change (16 → 32) must change the cache key: {} vs {}",
        keys_16[0], keys_32[0],
    );
}

#[test]
fn near_identical_sizes_collide_within_q8_quantum() {
    // The size quantisation matches `cache::GlyphKey::size_q8`
    // (`(size_px * 256.0).round()`), so two requests within ~1/256 px
    // of each other MUST hit the same cache slot — otherwise the
    // raster cache fragments with one entry per float bit pattern.
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("parses");
    let k1 = cache_keys(face, "H", 16.0);
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("parses");
    let k2 = cache_keys(face, "H", 16.0 + 1.0 / 1024.0);
    assert_eq!(
        k1, k2,
        "sub-q8 size deltas should collide: {k1:?} vs {k2:?}"
    );
}

#[test]
fn distinct_fonts_produce_distinct_cache_keys() {
    // DejaVuSans and DejaVuSansMono ship different glyph designs for
    // 'H'; even when their gid happens to match, the face_stable_id
    // input must keep their cache keys disjoint.
    const SANS: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
    let mono = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("mono parses");
    let sans = Face::from_ttf_bytes(SANS.to_vec()).expect("sans parses");
    assert_ne!(
        mono.stable_id(),
        sans.stable_id(),
        "two distinct fonts must have distinct stable_ids",
    );
    let mono_keys = cache_keys(mono, "H", 32.0);
    let sans_keys = cache_keys(sans, "H", 32.0);
    assert_ne!(
        mono_keys, sans_keys,
        "different fonts must produce different cache keys for 'H': {mono_keys:?} vs {sans_keys:?}",
    );
}
