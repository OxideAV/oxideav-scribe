//! `layout::wrap_lines` fills lines from per-token widths plus the
//! pair interaction at each joint instead of shaping every candidate
//! line whole. These tests hold it to the definition it replaces: the
//! lines must be exactly those found by shaping each whole candidate.

use oxideav_scribe::{Face, Shaper, layout};

const DEJAVU: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");
const INTER: &[u8] = include_bytes!("fixtures/InterVariable.ttf");
const SOURCE_SANS: &[u8] = include_bytes!("fixtures/SourceSans3-Regular.otf");
const NOTO_ARABIC: &[u8] = include_bytes!("fixtures/NotoSansArabic-Regular.ttf");

const LATIN: &str = "The quick brown fox jumps over the lazy dog. Sphinx of black quartz, \
judge my vow! Pack my box with five dozen liquor jugs; how vexingly quick daft zebras jump. \
Office affluent fjord waffle — 0123456789. To Te Ty AV, WA. \"Yes,\" T. said; (r.) f' L' \
P. A-Type; Tr. Ws Yo.  Double  spaces\tand a tab. A_very_long_unbreakable_token_that_must_\
be_cut_by_the_hard_break_path_because_it_is_wider_than_narrow_columns.";
const ARABIC: &str = "الخط العربي فن وتصميم الكتابة في مختلف اللغات التي تستعمل الحروف \
العربية. تتميز الكتابة العربية بكونها متصلة مما يجعلها قابلة لاكتساب أشكال هندسية مختلفة.";
const MIXED: &str = "Latin and עברית and العربية with é e\u{301} q\u{301} marks, 12 345 numbers.";

fn width(face: &Face, s: &str, size: f32) -> f32 {
    layout::run_width(&Shaper::shape(face, s, size).unwrap())
}

/// The definition: grow the line a whitespace-led word at a time,
/// shaping the whole candidate; break when it no longer fits; cut an
/// over-wide first word at the longest fitting prefix.
fn reference_wrap(face: &Face, text: &str, size: f32, max: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut words: Vec<String> = Vec::new();
        let mut buf = String::new();
        let mut in_word = false;
        for ch in paragraph.chars() {
            if ch.is_whitespace() {
                if in_word {
                    words.push(std::mem::take(&mut buf));
                    in_word = false;
                }
            } else {
                in_word = true;
            }
            buf.push(ch);
        }
        if !buf.is_empty() {
            words.push(buf);
        }
        let mut current = String::new();
        for word in words {
            let candidate = if current.is_empty() {
                word.trim_start().to_string()
            } else {
                format!("{current}{word}")
            };
            let w = width(face, &candidate, size);
            if w <= max || current.is_empty() {
                current = candidate;
                if w > max {
                    let chars: Vec<char> = current.chars().collect();
                    let mut good = 0;
                    for n in 1..=chars.len() {
                        let prefix: String = chars[..n].iter().collect();
                        if width(face, &prefix, size) > max {
                            break;
                        }
                        good = n;
                    }
                    let good = good.max(1).min(chars.len());
                    lines.push(chars[..good].iter().collect());
                    current = chars[good..].iter().collect();
                }
            } else {
                lines.push(current);
                current = word.trim_start().to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}

#[test]
fn wrapping_matches_whole_candidate_shaping() {
    let faces = [
        (
            "DejaVu Sans",
            Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap(),
        ),
        ("Inter", Face::from_ttf_bytes(INTER.to_vec()).unwrap()),
        (
            "Source Sans 3",
            Face::from_otf_bytes(SOURCE_SANS.to_vec()).unwrap(),
        ),
        (
            "Noto Sans Arabic",
            Face::from_ttf_bytes(NOTO_ARABIC.to_vec()).unwrap(),
        ),
    ];
    let texts = [LATIN, ARABIC, MIXED, "one\n\ntwo  three\n"];
    let mut compared = 0;
    for (name, face) in &faces {
        for text in texts {
            for (size, max) in [
                (16.0, 60.0),
                (16.0, 137.5),
                (16.0, 300.0),
                (11.0, 222.0),
                (24.0, 480.0),
            ] {
                let got = layout::wrap_lines(face, text, size, max).unwrap();
                let want = reference_wrap(face, text, size, max);
                assert_eq!(got, want, "{name}, size {size}, width {max}");
                compared += got.len();
            }
        }
    }
    assert!(compared > 500, "compared only {compared} lines");
}

#[test]
fn every_line_fits_or_is_a_single_overwide_piece() {
    let face = Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap();
    for max in [80.0, 200.0, 640.0] {
        for line in layout::wrap_lines(&face, LATIN, 16.0, max).unwrap() {
            let w = width(&face, &line, 16.0);
            assert!(
                w <= max + 0.01 || !line.trim().contains(' '),
                "{line:?} is {w} wide for {max}"
            );
        }
    }
}
