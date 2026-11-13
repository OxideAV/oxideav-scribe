//! Output identity across the face-lifetime parsed-font handle and the
//! outline cache: every glyph path, glyph node and shaping result on
//! the in-tree fixtures must be exactly what the re-parse-per-call
//! implementation produced. The pinned digests were computed with that
//! implementation (64-bit FNV-1a over the `Debug` rendering, which
//! prints every `f32` in shortest round-trip form, so equal digests
//! mean bit-equal coordinates).

use oxideav_scribe::{Face, FaceChain, OutlineCache, Shaper};
use std::sync::Arc;

mod font_fixtures;

const DEJAVU: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
const DEJAVU_MONO: &[u8] = include_bytes!("fixtures/DejaVuSansMono.ttf");
const DEJAVU_MONO_OBLIQUE: &[u8] = include_bytes!("fixtures/DejaVuSansMono-Oblique.ttf");
const INTER: &[u8] = include_bytes!("fixtures/InterVariable.ttf");
const SOURCE_SANS: &[u8] = include_bytes!("fixtures/SourceSans3-Regular.otf");

const LATIN: &str = "Sphinx of black quartz, judge my vow! Office affluent fjord — AV To 0123.";
const ARABIC: &str = "الخط العربي فن وتصميم الكتابة لا إله";
const MIXED: &str = "Hello שלום مرحبا Ελληνικά кириллица é ế";

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn feed(&mut self, v: &impl std::fmt::Debug) {
        use std::fmt::Write;
        struct Sink<'a>(&'a mut u64);
        impl Write for Sink<'_> {
            fn write_str(&mut self, s: &str) -> std::fmt::Result {
                for b in s.bytes() {
                    *self.0 ^= b as u64;
                    *self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
                }
                Ok(())
            }
        }
        write!(Sink(&mut self.0), "{v:?};").unwrap();
    }
}

fn glyph_count(face: &Face) -> u16 {
    match face.kind() {
        oxideav_scribe::FaceKind::Otf => face.with_otf_font(|f| f.glyph_count()).unwrap(),
        _ => face.with_font(|f| f.glyph_count()).unwrap(),
    }
}

/// Every glyph's native path, plus its node at three sizes for a
/// stride of the glyph set.
fn outline_digest(face: &Face) -> u64 {
    let mut h = Fnv::new();
    let n = glyph_count(face);
    for gid in 0..n {
        h.feed(&face.glyph_path(gid));
        if gid % 5 == 0 {
            for size in [12.0_f32, 16.0, 37.5] {
                h.feed(&face.glyph_node(gid, size));
            }
        }
    }
    // Out-of-range ids and degenerate sizes.
    h.feed(&face.glyph_path(n));
    h.feed(&face.glyph_path(u16::MAX));
    h.feed(&face.glyph_node(n, 16.0));
    h.feed(&face.glyph_node(3, 0.0));
    h.feed(&face.glyph_node(3, f32::NAN));
    h.0
}

fn shaping_digest(face: Face) -> u64 {
    let mut h = Fnv::new();
    for text in [LATIN, ARABIC, MIXED] {
        h.feed(&Shaper::shape(&face, text, 16.0).ok());
        h.feed(&face.shape_text(text, &[*b"liga", *b"smcp"]));
        h.feed(&face.shape_text_itemized(text, &[*b"liga"]));
        h.feed(&face.position_text(text, 22.0, &[*b"liga", *b"frac"]));
        h.feed(&face.position_text_itemized(text, 13.0, &[]));
        h.feed(&face.script_run_tags(text));
    }
    h.feed(&face.gsub_features_for_script(*b"latn", None));
    h.feed(&face.gpos_features_for_script(*b"latn", None));
    h.feed(&face.layout_features_for_script(*b"arab", None));
    h.feed(&face.is_variable());
    h.feed(&face.variation_axes());
    h.feed(&face.named_instances().len());
    h.feed(&face.has_color_bitmaps());
    h.feed(&face.color_strike_sizes());
    h.feed(&face.has_base_table());
    h.feed(&face.glyph_name(40));
    h.feed(&face.stable_id());
    let chain = FaceChain::new(face);
    for text in [LATIN, ARABIC, MIXED] {
        h.feed(&chain.shape(text, 16.0).ok());
        h.feed(&chain.shape_with_features(text, 16.0, &[*b"smcp"]).ok());
        h.feed(&Shaper::shape_to_paths(&chain, text, 19.0));
    }
    h.0
}

fn check(label: &str, got: u64, want: u64) {
    assert_eq!(
        got, want,
        "{label}: digest {got:#018x} differs from the pinned {want:#018x}"
    );
}

#[test]
fn dejavu_sans_outlines_are_identical() {
    let face = Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap();
    check(
        "DejaVuSans outlines",
        outline_digest(&face),
        0x4dd3_e25d_4430_ce2b,
    );
}

#[test]
fn dejavu_mono_outlines_are_identical() {
    let face = Face::from_ttf_bytes(DEJAVU_MONO.to_vec()).unwrap();
    check(
        "DejaVuSansMono outlines",
        outline_digest(&face),
        0xe518_aa3b_cafe_5713,
    );
    let face = Face::from_ttf_bytes(DEJAVU_MONO_OBLIQUE.to_vec()).unwrap();
    check(
        "DejaVuSansMono-Oblique outlines",
        outline_digest(&face),
        0x5798_21d2_fc70_b65c,
    );
}

#[test]
fn inter_variable_outlines_are_identical_per_instance() {
    let mut face = Face::from_ttf_bytes(INTER.to_vec()).unwrap();
    let default = outline_digest(&face);
    check("Inter default instance", default, 0x41dc_7753_5405_533f);
    face.set_variation_coords(&[700.0]).unwrap();
    check(
        "Inter wght=700",
        outline_digest(&face),
        0xd457_47ff_79b7_9d2e,
    );
    face.set_variation_coords(&[250.0, 30.0]).unwrap();
    check(
        "Inter wght=250 opsz=30",
        outline_digest(&face),
        0x90c4_66ca_05e2_1719,
    );
    // Returning to the default instance returns the default outlines.
    face.clear_variation_coords();
    check("Inter cleared", outline_digest(&face), default);
}

#[test]
fn source_sans_cff_outlines_are_identical() {
    let face = Face::from_otf_bytes(SOURCE_SANS.to_vec()).unwrap();
    check(
        "SourceSans3 outlines",
        outline_digest(&face),
        0xa252_ffe1_4c58_de7c,
    );
}

#[test]
fn shaping_results_are_identical() {
    check(
        "DejaVuSans shaping",
        shaping_digest(Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap()),
        0x5244_07ec_dcea_ba95,
    );
    check(
        "Inter shaping",
        shaping_digest(Face::from_ttf_bytes(INTER.to_vec()).unwrap()),
        0xa8aa_c244_b194_7a12,
    );
    let mut bold = Face::from_ttf_bytes(INTER.to_vec()).unwrap();
    bold.set_variation_coords(&[700.0]).unwrap();
    check(
        "Inter wght=700 shaping",
        shaping_digest(bold),
        0xae5d_ef64_6886_e612,
    );
    check(
        "SourceSans3 shaping",
        shaping_digest(Face::from_otf_bytes(SOURCE_SANS.to_vec()).unwrap()),
        0x73f7_6100_83c7_f099,
    );
}

#[test]
fn builder_installed_instance_matches_and_restores() {
    let mut chain = FaceChain::new(Face::from_ttf_bytes(INTER.to_vec()).unwrap());
    let mut h = Fnv::new();
    h.feed(&Shaper::shape_to_paths(&chain, LATIN, 24.0));
    let bold = Shaper::with_variation_coords(vec![900.0]);
    h.feed(&bold.shape_to_paths(&mut chain, LATIN, 24.0));
    h.feed(&bold.shape(&mut chain, LATIN, 24.0).ok());
    h.feed(&Shaper::shape_to_paths(&chain, LATIN, 24.0));
    check("Inter builder sequence", h.0, 0x06ed_97a4_82bf_8691);
}

#[test]
fn colour_emoji_nodes_are_identical() {
    let Some(bytes) = font_fixtures::load_fixture(&font_fixtures::NOTO_COLOR_EMOJI_TTF) else {
        return;
    };
    let face = Face::from_ttf_bytes(bytes).unwrap();
    let mut h = Fnv::new();
    for ch in ['🎉', '😀', '😂', 'A', ' '] {
        let gid = face.with_font(|f| f.glyph_index(ch)).unwrap().unwrap_or(0);
        h.feed(&gid);
        h.feed(&face.glyph_path(gid));
        for size in [16.0_f32, 32.0, 109.0] {
            h.feed(&face.glyph_node(gid, size));
        }
    }
    h.feed(&face.has_color_bitmaps());
    h.feed(&face.color_strike_sizes());
    check("NotoColorEmoji nodes", h.0, 0xe2bf_db99_f6ec_baa6);
}

#[test]
fn cjk_collection_outlines_are_identical() {
    let Some(bytes) = font_fixtures::load_fixture(&font_fixtures::NOTO_SANS_CJK_MEDIUM_TTC) else {
        return;
    };
    let Ok(face) = Face::from_ttc_bytes(bytes, 0) else {
        return;
    };
    let mut h = Fnv::new();
    for ch in "日本語漢字ひらがなカタカナ中文한국어。、A".chars() {
        let gid = face.with_font(|f| f.glyph_index(ch)).unwrap().unwrap_or(0);
        h.feed(&gid);
        h.feed(&face.glyph_path(gid));
        h.feed(&face.glyph_node(gid, 20.0));
    }
    let chain = FaceChain::new(Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap()).push_fallback(face);
    h.feed(&chain.shape("日本語 and Latin 한국어", 16.0).ok());
    h.feed(&Shaper::shape_to_paths(&chain, "日本語 mixed 中文", 18.0));
    check("NotoSansCJK nodes", h.0, 0xb3dc_86b4_1572_edf0);
}

/// The same pinned digests with an outline cache attached — cold,
/// then warm — and with one cache shared by every fixture face.
#[test]
fn outline_cache_reproduces_the_pinned_digests() {
    let cache = Arc::new(OutlineCache::default());
    let with_cache = |face: Face| face.with_outline_cache(cache.clone());

    let dejavu = with_cache(Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap());
    let mono = with_cache(Face::from_ttf_bytes(DEJAVU_MONO.to_vec()).unwrap());
    let cff = with_cache(Face::from_otf_bytes(SOURCE_SANS.to_vec()).unwrap());
    let mut inter = with_cache(Face::from_ttf_bytes(INTER.to_vec()).unwrap());
    for pass in ["cold", "warm"] {
        check(
            &format!("DejaVuSans outlines ({pass} cache)"),
            outline_digest(&dejavu),
            0x4dd3_e25d_4430_ce2b,
        );
        check(
            &format!("DejaVuSansMono outlines ({pass} cache)"),
            outline_digest(&mono),
            0xe518_aa3b_cafe_5713,
        );
        check(
            &format!("SourceSans3 outlines ({pass} cache)"),
            outline_digest(&cff),
            0xa252_ffe1_4c58_de7c,
        );
        inter.clear_variation_coords();
        check(
            &format!("Inter default instance ({pass} cache)"),
            outline_digest(&inter),
            0x41dc_7753_5405_533f,
        );
        inter.set_variation_coords(&[700.0]).unwrap();
        check(
            &format!("Inter wght=700 ({pass} cache)"),
            outline_digest(&inter),
            0xd457_47ff_79b7_9d2e,
        );
    }
    let stats = cache.stats();
    assert!(stats.hits > 0 && stats.bytes <= cache.budget_bytes());

    check(
        "DejaVuSans shaping (cache)",
        shaping_digest(with_cache(Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap())),
        0x5244_07ec_dcea_ba95,
    );
    let mut bold = with_cache(Face::from_ttf_bytes(INTER.to_vec()).unwrap());
    bold.set_variation_coords(&[700.0]).unwrap();
    check(
        "Inter wght=700 shaping (cache)",
        shaping_digest(bold),
        0xae5d_ef64_6886_e612,
    );
}
