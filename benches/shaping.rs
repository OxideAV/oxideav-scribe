//! Shaping and paragraph-layout timings.
//!
//! `cargo bench --bench shaping` — std-only wall-clock timing (no bench
//! framework dependency); each line is the median of several batches.
//! Pass a substring to run only matching cases.
//!
//! * **warm** — the face has shaped before: lookup plans and the font
//!   parser's lookup accelerators are built.
//! * **cold** — a fresh `Face` per iteration: construction plus the
//!   first shaping call, which builds whatever the text needs.
//! * **document** — `layout::shape_paragraphs` over a 100 000-character
//!   text (wrap to 480 px, bidi, shape every line).
//! * **outlines** — `Face::glyph_path` / `glyph_node` for every glyph
//!   of a paragraph; a per-glyph cost near the outline walk itself
//!   shows nothing re-parses the font.

use oxideav_scribe::{Face, FaceChain, layout};
use std::hint::black_box;
use std::time::Instant;

const DEJAVU: &[u8] = include_bytes!("../tests/fixtures/DejaVuSans.ttf");
const INTER: &[u8] = include_bytes!("../tests/fixtures/InterVariable.ttf");
const SOURCE_SANS: &[u8] = include_bytes!("../tests/fixtures/SourceSans3-Regular.otf");
const NOTO_ARABIC: &[u8] = include_bytes!("../tests/fixtures/NotoSansArabic-Regular.ttf");

const LATIN: &str = "The quick brown fox jumps over the lazy dog. Sphinx of black quartz, \
judge my vow! Pack my box with five dozen liquor jugs; how vexingly quick daft zebras jump. \
Office affluent fjord waffle — 0123456789. A wizard's job is to vex chumps quickly in fog; \
the five boxing wizards jump quickly, and sixty zippers were quickly picked from the woven \
jute bag. Typography is the art and technique of arranging type to make written language \
legible, readable and appealing when displayed.";
const ARABIC: &str = "الخط العربي فن وتصميم الكتابة في مختلف اللغات التي تستعمل الحروف \
العربية. تتميز الكتابة العربية بكونها متصلة مما يجعلها قابلة لاكتساب أشكال هندسية مختلفة \
من خلال المد والرجع والاستدارة والتزوية والتشابك والتداخل والتركيب. يقترن فن الخط \
بالزخرفة العربية حيث يستعمل لتزيين المساجد والقصور، كما أنه يستعمل في تحلية المخطوطات \
والكتب وخاصة نسخ القرآن الكريم.";

fn median_ns(mut f: impl FnMut(), batches: usize, iters: usize) -> f64 {
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

struct Bench {
    filter: Option<String>,
}

impl Bench {
    fn run(&self, name: &str, units: usize, unit: &str, iters: usize, f: impl FnMut()) {
        if let Some(s) = &self.filter
            && !name.contains(s.as_str())
        {
            return;
        }
        let ns = median_ns(f, 9, iters);
        println!(
            "{name:<46} {:>11.1} µs/iter {:>9.3} µs/{unit} ({units} {unit}s)",
            ns / 1e3,
            ns / 1e3 / units.max(1) as f64
        );
    }
}

fn document(paragraph: &str, chars: usize) -> String {
    let mut doc = String::new();
    let mut count = 0;
    while count < chars {
        doc.push_str(paragraph);
        doc.push('\n');
        count += paragraph.chars().count() + 1;
    }
    doc
}

fn main() {
    let bench = Bench {
        filter: std::env::args().skip(1).find(|a| !a.starts_with('-')),
    };
    let ttf = |b: &[u8]| Face::from_ttf_bytes(b.to_vec()).unwrap();
    let otf = |b: &[u8]| Face::from_otf_bytes(b.to_vec()).unwrap();

    // --- shaping, warm and cold --------------------------------------
    type MakeFace<'a> = &'a dyn Fn() -> Face;
    let cases: [(&str, MakeFace<'_>, &str); 6] = [
        ("Latin, DejaVu Sans", &|| ttf(DEJAVU), LATIN),
        ("Latin, Inter Variable", &|| ttf(INTER), LATIN),
        ("Latin, Source Sans 3 (CFF)", &|| otf(SOURCE_SANS), LATIN),
        ("Arabic, DejaVu Sans", &|| ttf(DEJAVU), ARABIC),
        ("Arabic, Noto Sans Arabic", &|| ttf(NOTO_ARABIC), ARABIC),
        ("Latin words, DejaVu Sans", &|| ttf(DEJAVU), ""),
    ];
    for (label, make, text) in cases {
        if text.is_empty() {
            // One call per word: the shape of UI labels and of
            // word-at-a-time measurement.
            let chain = FaceChain::new(make());
            let words: Vec<&str> = LATIN.split(' ').collect();
            let glyphs: usize = words
                .iter()
                .map(|w| chain.shape(w, 16.0).map_or(0, |g| g.len()))
                .sum();
            bench.run(&format!("shape warm: {label}"), glyphs, "glyph", 20, || {
                for w in &words {
                    black_box(chain.shape(black_box(w), 16.0).unwrap_or_default());
                }
            });
            continue;
        }
        let chain = FaceChain::new(make());
        let Ok(shaped) = chain.shape(text, 16.0) else {
            println!("shape: {label}: not shapeable");
            continue;
        };
        let glyphs = shaped.len();
        bench.run(&format!("shape warm: {label}"), glyphs, "glyph", 20, || {
            black_box(chain.shape(black_box(text), 16.0).unwrap_or_default());
        });
        bench.run(&format!("shape cold: {label}"), glyphs, "glyph", 3, || {
            let chain = FaceChain::new(make());
            black_box(chain.shape(black_box(text), 16.0).unwrap_or_default());
        });
    }

    // --- paragraph layout of a 100 000-character document ------------
    for (label, face, paragraph) in [
        ("Latin, DejaVu Sans", ttf(DEJAVU), LATIN),
        ("Latin, Inter Variable", ttf(INTER), LATIN),
        ("Arabic, Noto Sans Arabic", ttf(NOTO_ARABIC), ARABIC),
    ] {
        let doc = document(paragraph, 100_000);
        let chars = doc.chars().count();
        let chain = FaceChain::new(face);
        let lines = match layout::shape_paragraphs(&chain, &doc, 16.0, 480.0, None) {
            Ok(p) => p.iter().map(|p| p.lines.len()).sum::<usize>(),
            Err(_) => {
                println!("document: {label}: not shapeable");
                continue;
            }
        };
        bench.run(
            &format!("document 100k ({lines} lines): {label}"),
            chars,
            "char",
            1,
            || {
                black_box(
                    layout::shape_paragraphs(&chain, black_box(&doc), 16.0, 480.0, None).ok(),
                );
            },
        );
    }

    // --- glyph outline retrieval --------------------------------------
    for (label, face) in [
        ("DejaVu Sans", ttf(DEJAVU)),
        ("Inter Variable", ttf(INTER)),
        ("Source Sans 3 (CFF)", otf(SOURCE_SANS)),
    ] {
        let chain = FaceChain::new(face);
        let Ok(shaped) = chain.shape(LATIN, 16.0) else {
            continue;
        };
        let gids: Vec<u16> = shaped.iter().map(|g| g.glyph_id).collect();
        let face = chain.primary();
        bench.run(
            &format!("outlines glyph_path: {label}"),
            gids.len(),
            "glyph",
            20,
            || {
                for &g in &gids {
                    black_box(face.glyph_path(g));
                }
            },
        );
        bench.run(
            &format!("outlines glyph_node: {label}"),
            gids.len(),
            "glyph",
            20,
            || {
                for &g in &gids {
                    black_box(face.glyph_node(g, 16.0));
                }
            },
        );
    }
}
