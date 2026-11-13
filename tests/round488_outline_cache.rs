//! The optional outline cache: output is identical with and without
//! it, a hit is never a glyph of another face or another variation
//! instance, the byte budget holds, and hostile fonts drawn through a
//! shared cache behave exactly as they do without one.

use oxideav_core::PathCommand;
use oxideav_scribe::{Face, FaceChain, FaceKind, OutlineCache, Shaper};
use std::sync::Arc;

mod font_fixtures;

const DEJAVU: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
const DEJAVU_MONO: &[u8] = include_bytes!("fixtures/DejaVuSansMono.ttf");
const INTER: &[u8] = include_bytes!("fixtures/InterVariable.ttf");
const SOURCE_SANS: &[u8] = include_bytes!("fixtures/SourceSans3-Regular.otf");

fn ttf(bytes: &[u8]) -> Face {
    Face::from_ttf_bytes(bytes.to_vec()).expect("fixture parses")
}

/// `Face::glyph_path` as comparable commands.
fn gp(face: &Face, gid: u16) -> Option<Vec<PathCommand>> {
    face.glyph_path(gid).map(|p| p.commands)
}

fn glyph_count(face: &Face) -> u16 {
    match face.kind() {
        FaceKind::Otf => face.with_otf_font(|f| f.glyph_count()).unwrap(),
        _ => face.with_font(|f| f.glyph_count()).unwrap(),
    }
}

/// `cached` must answer every outline query exactly as `plain` (a
/// cache-less face over the same bytes and instance) does.
fn assert_same_outlines(label: &str, cached: &Face, plain: &Face, stride: usize) {
    let n = glyph_count(plain);
    for gid in (0..=n).step_by(stride) {
        let want = gp(plain, gid);
        assert_eq!(gp(cached, gid), want, "{label}: glyph_path({gid})");
        assert_eq!(
            cached.glyph_path_shared(gid).map(|p| p.commands.clone()),
            want,
            "{label}: glyph_path_shared({gid})"
        );
        for size in [9.0_f32, 16.0, 64.5] {
            assert_eq!(
                format!("{:?}", cached.glyph_node(gid, size)),
                format!("{:?}", plain.glyph_node(gid, size)),
                "{label}: glyph_node({gid}, {size})"
            );
        }
    }
}

#[test]
fn cached_and_uncached_outlines_are_identical() {
    let cache = Arc::new(OutlineCache::default());
    for (label, bytes) in [("DejaVuSans", DEJAVU), ("DejaVuSansMono", DEJAVU_MONO)] {
        let plain = ttf(bytes);
        let cached = ttf(bytes).with_outline_cache(cache.clone());
        // Cold, then warm.
        assert_same_outlines(label, &cached, &plain, 3);
        assert_same_outlines(label, &cached, &plain, 3);
    }
    let plain = Face::from_otf_bytes(SOURCE_SANS.to_vec()).unwrap();
    let cached = Face::from_otf_bytes(SOURCE_SANS.to_vec())
        .unwrap()
        .with_outline_cache(cache.clone());
    assert_same_outlines("SourceSans3", &cached, &plain, 3);
    assert_same_outlines("SourceSans3", &cached, &plain, 3);
    let s = cache.stats();
    assert!(s.hits > s.misses, "warm passes must hit: {s:?}");
    assert!(s.bytes <= cache.budget_bytes());
}

#[test]
fn a_warm_cache_decodes_nothing() {
    let cache = Arc::new(OutlineCache::default());
    let face = ttf(DEJAVU).with_outline_cache(cache.clone());
    assert!(Arc::ptr_eq(face.outline_cache().unwrap(), &cache));
    let gids: Vec<u16> = "Hello, world"
        .chars()
        .map(|c| face.with_font(|f| f.glyph_index(c)).unwrap().unwrap())
        .collect();
    for &g in &gids {
        face.glyph_node(g, 14.0);
    }
    let cold = cache.stats();
    // Any size, any number of times: no further decode.
    for size in [10.0_f32, 14.0, 28.0, 96.0] {
        for &g in &gids {
            face.glyph_node(g, size);
            gp(&face, g);
        }
    }
    let warm = cache.stats();
    assert_eq!(warm.misses, cold.misses);
    assert_eq!(warm.entries, cold.entries);
    assert_eq!(warm.hits, cold.hits + 8 * gids.len() as u64);
    // The shared path is the cache's own allocation.
    let a = face.glyph_path_shared(gids[0]).unwrap();
    let b = face.glyph_path_shared(gids[0]).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
}

/// Faces sharing one cache never see each other's glyphs — not two
/// different fonts, and not two faces built from the very same bytes
/// at different variation instances.
#[test]
fn faces_sharing_a_cache_do_not_alias() {
    let cache = Arc::new(OutlineCache::default());
    let sans = ttf(DEJAVU).with_outline_cache(cache.clone());
    let mono = ttf(DEJAVU_MONO).with_outline_cache(cache.clone());
    let regular = ttf(INTER).with_outline_cache(cache.clone());
    let mut bold = ttf(INTER).with_outline_cache(cache.clone());
    bold.set_variation_coords(&[14.0, 800.0]).unwrap();

    let sans_ref = ttf(DEJAVU);
    let mono_ref = ttf(DEJAVU_MONO);
    let regular_ref = ttf(INTER);
    let mut bold_ref = ttf(INTER);
    bold_ref.set_variation_coords(&[14.0, 800.0]).unwrap();

    let mut differing = 0;
    for round in 0..2 {
        for gid in (0..600u16).step_by(7) {
            // Interleave so every face looks up the same glyph id
            // back to back.
            let got = [
                gp(&sans, gid),
                gp(&mono, gid),
                gp(&regular, gid),
                gp(&bold, gid),
            ];
            let want = [
                gp(&sans_ref, gid),
                gp(&mono_ref, gid),
                gp(&regular_ref, gid),
                gp(&bold_ref, gid),
            ];
            assert_eq!(got, want, "round {round} glyph {gid}");
            if want[2] != want[3] {
                differing += 1;
            }
        }
    }
    // The instances really do differ, so aliasing would have shown.
    assert!(differing > 50, "only {differing} glyphs differ by instance");
}

/// One face moving between instances: each instance's outlines come
/// back exactly, including after returning to an earlier instance
/// whose entries are still cached.
#[test]
fn changing_the_instance_never_serves_a_stale_outline() {
    let cache = Arc::new(OutlineCache::default());
    let mut face = ttf(INTER).with_outline_cache(cache.clone());
    let mut reference = ttf(INTER);
    let gids: Vec<u16> = (0..400u16).step_by(11).collect();
    let instances: [&[f32]; 6] = [
        &[],
        &[14.0, 700.0],
        &[32.0, 100.0],
        &[14.0, 700.0],
        &[],
        &[20.0, 433.0],
    ];
    for (step, coords) in instances.iter().enumerate() {
        if coords.is_empty() {
            face.clear_variation_coords();
            reference.clear_variation_coords();
        } else {
            face.set_variation_coords(coords).unwrap();
            reference.set_variation_coords(coords).unwrap();
        }
        assert_eq!(face.variation_coords(), reference.variation_coords());
        for &gid in &gids {
            assert_eq!(
                gp(&face, gid),
                gp(&reference, gid),
                "step {step} {coords:?} glyph {gid}"
            );
            assert_eq!(
                format!("{:?}", face.glyph_node(gid, 21.0)),
                format!("{:?}", reference.glyph_node(gid, 21.0)),
            );
        }
    }
    // Steps 3 and 4 revisit instances 1 and 0: served from the cache.
    let s = cache.stats();
    assert!(
        s.hits >= 2 * gids.len() as u64,
        "revisited instances should hit: {s:?}"
    );
}

#[test]
fn a_tiny_budget_changes_nothing_but_the_hit_rate() {
    for budget in [0usize, 512, 4096, 64 << 10] {
        let cache = Arc::new(OutlineCache::new(budget));
        let cached = ttf(DEJAVU).with_outline_cache(cache.clone());
        let plain = ttf(DEJAVU);
        for pass in 0..2 {
            for gid in (0..3000u16).step_by(13) {
                assert_eq!(
                    gp(&cached, gid),
                    gp(&plain, gid),
                    "budget {budget} pass {pass} glyph {gid}"
                );
                let s = cache.stats();
                assert!(s.bytes <= budget, "budget {budget}: holds {}", s.bytes);
            }
        }
    }
}

#[test]
fn chain_wide_cache_leaves_shape_to_paths_unchanged() {
    let text = "Sphinx of black quartz — مرحبا بالعالم — fi fl ffi";
    let build = || FaceChain::new(ttf(DEJAVU_MONO)).push_fallback(ttf(DEJAVU));
    let plain = build();
    let mut cached = build();
    let cache = Arc::new(OutlineCache::default());
    cached.set_outline_cache(Some(cache.clone()));
    // Both chains are built from the same bytes, so even the
    // content-derived `Group::cache_key`s agree.
    let want = format!("{:?}", Shaper::shape_to_paths(&plain, text, 18.0));
    for _ in 0..3 {
        let got = format!("{:?}", Shaper::shape_to_paths(&cached, text, 18.0));
        assert_eq!(got, want);
    }
    assert!(cache.stats().hits > 0);
    // Detaching goes back to decoding per call, same output.
    cached.set_outline_cache(None);
    assert!(cached.face(0).outline_cache().is_none());
    let rebuilt = build().with_outline_cache(cache.clone());
    assert!(Arc::ptr_eq(
        rebuilt.face(1).outline_cache().unwrap(),
        &cache
    ));
    let hits = cache.stats().hits;
    let got = format!("{:?}", Shaper::shape_to_paths(&cached, text, 18.0));
    assert_eq!(got, want);
    assert_eq!(cache.stats().hits, hits);
}

#[test]
fn drawing_from_several_threads_through_one_cache() {
    let cache = Arc::new(OutlineCache::new(256 << 10));
    let faces = [
        ttf(DEJAVU).with_outline_cache(cache.clone()),
        ttf(DEJAVU_MONO).with_outline_cache(cache.clone()),
    ];
    let refs = [ttf(DEJAVU), ttf(DEJAVU_MONO)];
    let want: Vec<Vec<Option<Vec<PathCommand>>>> = refs
        .iter()
        .map(|f| (0..500u16).map(|g| gp(f, g)).collect())
        .collect();
    std::thread::scope(|s| {
        for t in 0..4u16 {
            let (faces, want) = (&faces, &want);
            s.spawn(move || {
                for i in 0..1500u16 {
                    let gid = (i * 13 + t * 7) % 500;
                    let which = ((i + t) % 2) as usize;
                    assert_eq!(
                        gp(&faces[which], gid),
                        want[which][gid as usize],
                        "thread {t} face {which} glyph {gid}"
                    );
                    assert!(
                        faces[which].glyph_node(gid, 12.0).is_some()
                            == want[which][gid as usize].is_some()
                    );
                }
            });
        }
    });
    assert!(cache.stats().bytes <= cache.budget_bytes());
}

// ---------- hostile fonts through the cached paths -----------------------

/// Deterministic splitmix64 so a failing run reproduces from its seed.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
}

/// Byte-mutated fonts, all drawn through ONE small shared cache (so
/// entries of many corrupt faces — most of them byte-for-byte equal
/// except for a few flips, with identical `stable_id`s — sit side by
/// side and keep getting evicted). Each face must get back exactly
/// what an uncached twin over the same mutated bytes decodes: never
/// another face's glyph, never another instance's.
#[test]
fn mutated_fonts_through_a_shared_cache_match_their_uncached_twins() {
    let cache = Arc::new(OutlineCache::new(96 << 10));
    let mut rng = Rng(0x0488_CAC4E);
    let mut drawn = 0u32;
    let mut kept: Vec<(Face, Face)> = Vec::new();
    for iter in 0..160u32 {
        let (source, variable) = match iter % 3 {
            0 => (DEJAVU_MONO, false),
            1 => (INTER, true),
            _ => (DEJAVU, false),
        };
        let mut bytes = source.to_vec();
        for _ in 0..1 + rng.below(5) {
            // Skew the flips toward the table directory and the early
            // tables, where they change structure rather than a pixel.
            let span = if rng.below(3) == 0 {
                bytes.len() as u64
            } else {
                4096
            };
            let off = rng.below(span) as usize;
            bytes[off] ^= (rng.below(255) + 1) as u8;
        }
        let (Ok(mut cached), Ok(mut plain)) = (
            Face::from_ttf_bytes(bytes.clone()),
            Face::from_ttf_bytes(bytes),
        ) else {
            continue;
        };
        cached.set_outline_cache(Some(cache.clone()));
        if variable && rng.below(2) == 0 {
            let coords = [14.0 + rng.below(18) as f32, 100.0 + rng.below(800) as f32];
            let a = cached.set_variation_coords(&coords);
            let b = plain.set_variation_coords(&coords);
            assert_eq!(a.is_ok(), b.is_ok());
        }
        let n = glyph_count(&plain).max(1);
        for _ in 0..40 {
            let gid = rng.below(n as u64 + 2) as u16;
            let want = gp(&plain, gid);
            assert_eq!(gp(&cached, gid), want, "iter {iter} glyph {gid}");
            assert_eq!(gp(&cached, gid), want, "iter {iter} glyph {gid} (warm)");
            assert_eq!(
                format!("{:?}", cached.glyph_node(gid, 15.0)),
                format!("{:?}", plain.glyph_node(gid, 15.0)),
                "iter {iter} glyph {gid} node"
            );
            drawn += 1;
        }
        assert!(cache.stats().bytes <= cache.budget_bytes());
        // Keep a few faces alive and re-query them later, after many
        // other faces have churned the cache.
        if iter % 8 == 0 {
            kept.push((cached, plain));
        }
        for (c, p) in &kept {
            let n = glyph_count(p).max(1);
            let gid = rng.below(n as u64) as u16;
            assert_eq!(gp(c, gid), gp(p, gid), "kept face glyph {gid}");
        }
    }
    assert!(drawn > 1000, "only {drawn} draws ran — mutations too harsh");
    // Dropping a face's entries early is always safe.
    for (c, p) in &kept {
        cache.forget_face(c.id());
        assert_eq!(gp(c, 1), gp(p, 1));
    }
}

/// Colour-bitmap glyphs through the cache: the decoded strike is kept
/// once per (glyph, strike) and resampled per size; nodes and bitmaps
/// are identical to the uncached face's. (Needs the pinned emoji
/// fixture; skipped when it is not cached.)
#[test]
fn colour_strikes_are_decoded_once_and_identical() {
    let Some(bytes) = font_fixtures::load_fixture(&font_fixtures::NOTO_COLOR_EMOJI_TTF) else {
        return;
    };
    let cache = Arc::new(OutlineCache::new(8 << 20));
    let plain = Face::from_ttf_bytes(bytes.clone()).unwrap();
    let cached = Face::from_ttf_bytes(bytes)
        .unwrap()
        .with_outline_cache(cache.clone());
    let gids: Vec<u16> = "🎉😀😂🥰👍"
        .chars()
        .map(|c| plain.with_font(|f| f.glyph_index(c)).unwrap().unwrap_or(0))
        .collect();
    let sizes = [12.0_f32, 16.0, 31.5, 64.0, 109.0, 200.0];
    for pass in 0..2 {
        for &gid in &gids {
            for size in sizes {
                assert_eq!(
                    format!("{:?}", cached.glyph_node(gid, size)),
                    format!("{:?}", plain.glyph_node(gid, size)),
                    "pass {pass} glyph {gid} size {size}"
                );
                assert_eq!(
                    format!("{:?}", cached.raster_color_glyph(gid, size)),
                    format!("{:?}", plain.raster_color_glyph(gid, size)),
                );
                assert_eq!(
                    format!("{:?}", cached.raster_color_glyph_at(gid, size)),
                    format!("{:?}", plain.raster_color_glyph_at(gid, size)),
                );
            }
        }
    }
    let s = cache.stats();
    // One decode per distinct (glyph, strike); everything else hits.
    let strikes = plain.color_strike_sizes().len().max(1) as u64;
    assert!(
        s.misses <= gids.len() as u64 * (strikes + 1),
        "decoded more than once per strike: {s:?}"
    );
    assert!(s.hits > 10 * s.misses, "{s:?}");
    assert!(s.bytes <= cache.budget_bytes());
    assert!(matches!(
        cached.raster_color_glyph(gids[0], 0.0),
        Err(oxideav_scribe::Error::InvalidSize)
    ));
}
