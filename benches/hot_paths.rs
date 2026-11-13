//! Hot-path timings for the per-glyph and per-paragraph entry points.
//!
//! `cargo bench --bench hot_paths` — std-only wall-clock timing (no
//! bench framework dependency). Each line is the median of several
//! batches; pass a substring to run only matching cases.
//!
//! The in-tree fixtures (DejaVu Sans, Inter Variable, Source Sans 3)
//! always run. The CJK and colour-emoji cases use the two large pinned
//! fixtures the `round5_*` integration tests download into
//! `$CARGO_TARGET_DIR/test-fixtures/fonts/`; they are skipped when the
//! files are absent (set `OXIDEAV_SCRIBE_BENCH_FONTS` to point at
//! another directory holding them).

use oxideav_scribe::{Face, FaceChain, OutlineCache, Shaper};
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

const DEJAVU: &[u8] = include_bytes!("../tests/fixtures/DejaVuSans.ttf");
const INTER: &[u8] = include_bytes!("../tests/fixtures/InterVariable.ttf");
const SOURCE_SANS: &[u8] = include_bytes!("../tests/fixtures/SourceSans3-Regular.otf");

const LATIN: &str = "The quick brown fox jumps over the lazy dog. Sphinx of black quartz, \
judge my vow! Pack my box with five dozen liquor jugs; how vexingly quick daft zebras jump. \
Office affluent fjord waffle — 0123456789.";
const ARABIC: &str = "الخط العربي فن وتصميم الكتابة في مختلف اللغات التي تستعمل الحروف \
العربية. تتميز الكتابة العربية بكونها متصلة مما يجعلها قابلة لاكتساب أشكال هندسية مختلفة.";
const CJK: &str = "日本語の文章を組むときは、漢字とひらがなとカタカナを混ぜて書きます。\
中文排版需要處理標點符號的位置。한국어 문장도 함께 넣어 봅니다。";
const EMOJI: &str = "🎉😀😁😂🤣😃😄😅😆😉😊😋😎😍😘🥰";

/// The reporter's frame: one text field plus thirty labelled buttons.
fn ui_frame_labels() -> Vec<String> {
    let mut v = vec![String::from(
        "Type here to search the document for a phrase…",
    )];
    for i in 0..30 {
        v.push(format!("Button label {i}"));
    }
    v
}

fn median_ns(mut f: impl FnMut(), batches: usize, iters: usize) -> f64 {
    // Warm-up.
    f();
    let mut samples: Vec<f64> = (0..batches)
        .map(|_| {
            let t = Instant::now();
            for _ in 0..iters {
                f();
            }
            t.elapsed().as_nanos() as f64 / iters as f64
        })
        .collect();
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

fn report(filter: &Option<String>, name: &str, units: usize, unit: &str, f: impl FnMut()) {
    if let Some(s) = filter
        && !name.contains(s.as_str())
    {
        return;
    }
    let ns = median_ns(f, 9, 5);
    println!(
        "{name:<44} {:>12.1} µs/iter {:>10.1} ns/{unit} ({units} {unit}s)",
        ns / 1e3,
        ns / units.max(1) as f64
    );
}

fn big_fixture(name: &str) -> Option<Vec<u8>> {
    let dir = std::env::var_os("OXIDEAV_SCRIBE_BENCH_FONTS")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("CARGO_TARGET_DIR")
                .map(|t| PathBuf::from(t).join("test-fixtures").join("fonts"))
        })?;
    std::fs::read(dir.join(name)).ok()
}

/// Shape once outside the timed region; returns `(face_idx, glyph_id)`.
fn shaped_gids(chain: &FaceChain, text: &str) -> Vec<(u16, u16)> {
    chain
        .shape(text, 16.0)
        .unwrap_or_default()
        .iter()
        .map(|g| (g.face_idx, g.glyph_id))
        .collect()
}

fn bench_chain(filter: &Option<String>, label: &str, chain: &mut FaceChain, text: &str) {
    bench_chain_once(filter, label, chain, text);
    chain.set_outline_cache(Some(Arc::new(OutlineCache::default())));
    bench_chain_once(filter, &format!("{label} [cache]"), chain, text);
    chain.set_outline_cache(None);
}

fn bench_chain_once(filter: &Option<String>, label: &str, chain: &FaceChain, text: &str) {
    let gids = shaped_gids(chain, text);
    let n = gids.len();
    report(filter, &format!("{label}: shape"), n, "glyph", || {
        black_box(chain.shape(black_box(text), 16.0).unwrap_or_default());
    });
    report(
        filter,
        &format!("{label}: draw (glyph_node)"),
        n,
        "glyph",
        || {
            for &(fi, gid) in &gids {
                black_box(chain.face(fi).glyph_node(gid, 16.0));
            }
        },
    );
    report(
        filter,
        &format!("{label}: shape_to_paths"),
        n,
        "glyph",
        || {
            black_box(Shaper::shape_to_paths(chain, black_box(text), 16.0));
        },
    );
}

fn main() {
    let filter = std::env::args().skip(1).find(|a| !a.starts_with('-'));

    // --- the reporter's frame: N glyph_node calls per frame ------------
    for (label, bytes, otf) in [
        ("ui-frame DejaVuSans", DEJAVU, false),
        ("ui-frame InterVariable", INTER, false),
        ("ui-frame SourceSans3 (CFF)", SOURCE_SANS, true),
    ] {
        let mut face = if otf {
            Face::from_otf_bytes(bytes.to_vec()).unwrap()
        } else {
            Face::from_ttf_bytes(bytes.to_vec()).unwrap()
        };
        let labels = ui_frame_labels();
        // Glyph ids via the per-face cmap, resolved outside the timing.
        let mut gids: Vec<u16> = Vec::new();
        for l in &labels {
            for ch in l.chars() {
                let gid = if otf {
                    face.with_otf_font(|f| f.glyph_index(ch)).unwrap()
                } else {
                    face.with_font(|f| f.glyph_index(ch)).unwrap()
                };
                gids.push(gid.unwrap_or(0));
            }
        }
        let n = gids.len();
        report(&filter, &format!("{label}: glyph_node"), n, "glyph", || {
            for &g in &gids {
                black_box(face.glyph_node(g, 14.0));
            }
        });
        report(&filter, &format!("{label}: glyph_path"), n, "glyph", || {
            for &g in &gids {
                black_box(face.glyph_path(g));
            }
        });
        face.set_outline_cache(Some(Arc::new(OutlineCache::default())));
        report(
            &filter,
            &format!("{label}: glyph_node [cache]"),
            n,
            "glyph",
            || {
                for &g in &gids {
                    black_box(face.glyph_node(g, 14.0));
                }
            },
        );
        report(
            &filter,
            &format!("{label}: glyph_path_shared [cache]"),
            n,
            "glyph",
            || {
                for &g in &gids {
                    black_box(face.glyph_path_shared(g));
                }
            },
        );
        if !otf {
            let mut chain = FaceChain::new(Face::from_ttf_bytes(bytes.to_vec()).unwrap());
            report(
                &filter,
                &format!("{label}: shape+draw frame"),
                n,
                "glyph",
                || {
                    for l in &labels {
                        black_box(Shaper::shape_to_paths(&chain, l, 14.0));
                    }
                },
            );
            chain.set_outline_cache(Some(Arc::new(OutlineCache::default())));
            report(
                &filter,
                &format!("{label}: shape+draw frame [cache]"),
                n,
                "glyph",
                || {
                    for l in &labels {
                        black_box(Shaper::shape_to_paths(&chain, l, 14.0));
                    }
                },
            );
        }
    }

    // --- construction ---------------------------------------------------
    report(&filter, "construct DejaVuSans", 1, "face", || {
        black_box(Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap());
    });
    report(&filter, "construct InterVariable", 1, "face", || {
        black_box(Face::from_ttf_bytes(INTER.to_vec()).unwrap());
    });

    // --- line wrapping --------------------------------------------------
    {
        let face = Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap();
        let long = [LATIN; 4].join(" ");
        let n = long.chars().count();
        report(&filter, "wrap 480px DejaVuSans", n, "char", || {
            black_box(oxideav_scribe::wrap_lines(&face, black_box(&long), 16.0, 480.0).unwrap());
        });
    }

    // --- paragraphs -----------------------------------------------------
    let mut dejavu = FaceChain::new(Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap());
    bench_chain(&filter, "latin DejaVuSans", &mut dejavu, LATIN);
    bench_chain(&filter, "arabic DejaVuSans", &mut dejavu, ARABIC);
    let mut inter = FaceChain::new(Face::from_ttf_bytes(INTER.to_vec()).unwrap());
    bench_chain(&filter, "latin InterVariable", &mut inter, LATIN);
    let mut inter_bold = FaceChain::new(Face::from_ttf_bytes(INTER.to_vec()).unwrap());
    let _ = inter_bold.face_mut(0).set_variation_coords(&[700.0]);
    bench_chain(
        &filter,
        "latin InterVariable wght=700",
        &mut inter_bold,
        LATIN,
    );

    match big_fixture("NotoSansCJK-Medium.ttc") {
        Some(bytes) => match Face::from_ttc_bytes(bytes, 0) {
            Ok(cjk) => {
                let mut chain = FaceChain::new(Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap())
                    .push_fallback(cjk);
                bench_chain(&filter, "cjk DejaVu+NotoSansCJK", &mut chain, CJK);
            }
            Err(e) => println!("cjk: fixture did not parse ({e:?}), skipped"),
        },
        None => println!("cjk: NotoSansCJK-Medium.ttc not cached, skipped"),
    }
    match big_fixture("NotoColorEmoji.ttf") {
        Some(bytes) => match Face::from_ttf_bytes(bytes) {
            Ok(emoji) => {
                let mut chain = FaceChain::new(Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap())
                    .push_fallback(emoji);
                bench_chain(&filter, "emoji DejaVu+NotoColorEmoji", &mut chain, EMOJI);
            }
            Err(e) => println!("emoji: fixture did not parse ({e:?}), skipped"),
        },
        None => println!("emoji: NotoColorEmoji.ttf not cached, skipped"),
    }
}
