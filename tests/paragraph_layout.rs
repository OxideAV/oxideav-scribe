//! `layout_text`: UAX #14 line breaking, alignment, justification,
//! tab stops, bidi paragraphs, vertical lines, cluster mapping.

use oxideav_scribe::{
    Align, Face, FaceChain, LaidOutLine, LayoutOptions, ShapeOptions, layout_text,
};

const DEJAVU: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn chain() -> FaceChain {
    FaceChain::new(Face::from_ttf_bytes(DEJAVU.to_vec()).unwrap())
}

fn line_text<'a>(text: &'a str, line: &LaidOutLine) -> &'a str {
    &text[line.text_range.clone()]
}

fn advance(line: &LaidOutLine) -> f32 {
    line.glyphs.iter().map(|g| g.x_advance).sum()
}

const PARAGRAPH: &str = "Typography is the art and technique of arranging type to make written \
language legible, readable and appealing when displayed. The arrangement of type involves \
selecting typefaces, point sizes, line lengths, line-spacing and letter-spacing.";

#[test]
fn lines_fit_tile_the_text_and_break_at_opportunities() {
    let chain = chain();
    for max in [150.0, 200.0, 333.0, 640.0] {
        let lines = layout_text(&chain, PARAGRAPH, 16.0, &LayoutOptions::new(max)).unwrap();
        assert!(lines.len() > 1);
        // The lines tile the paragraph.
        let mut at = 0;
        for line in &lines {
            assert_eq!(line.text_range.start, at);
            at = line.text_range.end;
            assert!(line.extent <= max + 0.01, "{} > {max}", line.extent);
            assert_eq!(line.base_level, 0);
            assert_eq!(line.paragraph, 0);
        }
        assert_eq!(at, PARAGRAPH.len());
        // Every line but the last ends at a space or after a hyphen,
        // and no line starts with a space.
        for line in &lines[..lines.len() - 1] {
            let t = line_text(PARAGRAPH, line);
            assert!(t.ends_with(' ') || t.ends_with('-'), "{t:?}");
            assert!(!line.forced_break);
        }
        assert!(
            lines
                .iter()
                .all(|l| !line_text(PARAGRAPH, l).starts_with(' '))
        );
        assert!(lines.last().unwrap().forced_break);
        // Greedy: the first word of the next line would not have fit.
        for pair in lines.windows(2) {
            let next_word = line_text(PARAGRAPH, &pair[1])
                .split_inclusive([' ', '-'])
                .next()
                .unwrap()
                .trim_end();
            let with = format!("{}{next_word}", line_text(PARAGRAPH, &pair[0]));
            let w: f32 = chain
                .shape(&with, 16.0)
                .unwrap()
                .iter()
                .map(|g| g.x_advance)
                .sum();
            assert!(w > max, "{with:?} ({w}) would have fit in {max}");
        }
    }
}

#[test]
fn breaks_follow_the_line_breaking_algorithm_not_just_spaces() {
    let chain = chain();
    let width = |t: &str| -> f32 {
        chain
            .shape(t, 16.0)
            .unwrap()
            .iter()
            .map(|g| g.x_advance)
            .sum()
    };
    let lay = |text: &str, max: f32| -> Vec<String> {
        layout_text(&chain, text, 16.0, &LayoutOptions::new(max))
            .unwrap()
            .iter()
            .map(|l| line_text(text, l).to_string())
            .collect()
    };
    // A hyphen offers a break after it.
    let text = "well-known";
    assert_eq!(
        lay(text, width("well-").max(width("known")) + 1.0),
        ["well-", "known"]
    );
    // A no-break space does not: the pair moves to the next line whole.
    let text = "xx aaa\u{a0}bbb yy";
    let parts = lay(text, width("aaa\u{a0}bbb") + 1.0);
    assert_eq!(parts, ["xx ", "aaa\u{a0}bbb ", "yy"]);
    // An ordinary space in the same place does.
    let text = "xx aaa bbb yy";
    assert_eq!(lay(text, width("aaa\u{a0}bbb") + 1.0)[0], "xx aaa ");
    // A solidus offers a break after it.
    let text = "either/or";
    assert_eq!(lay(text, width("either/") + 1.0), ["either/", "or"]);
    assert_eq!(lay(text, width("either/or") + 1.0), ["either/or"]);
    // Ideographs break between any two of them, but not before a
    // closing mark.
    let text = "\u{65e5}\u{672c}\u{8a9e}\u{3002}";
    let parts = lay(text, width("\u{8a9e}\u{3002}") + 1.0);
    assert_eq!(parts, ["\u{65e5}\u{672c}", "\u{8a9e}\u{3002}"]);
    // One ideograph too narrow for the pair "ideograph + full stop":
    // the full stop still stays with it on the next line.
    let text = "\u{65e5}\u{672c}\u{672c}\u{3002}";
    let parts = lay(text, width("\u{65e5}\u{672c}\u{672c}") + 1.0);
    assert_eq!(parts, ["\u{65e5}\u{672c}", "\u{672c}\u{3002}"]);
    // A line separator forces a break inside a paragraph.
    let text = "one\u{2028}two three";
    let lines = layout_text(&chain, text, 16.0, &LayoutOptions::new(0.0)).unwrap();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].forced_break && lines[0].paragraph == 0 && lines[1].paragraph == 0);
    assert_eq!(line_text(text, &lines[1]), "two three");
}

#[test]
fn overlong_segments_are_cut_between_clusters() {
    let chain = chain();
    let text = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let lines = layout_text(&chain, text, 16.0, &LayoutOptions::new(50.0)).unwrap();
    assert!(lines.len() > 3);
    let total: usize = lines.iter().map(|l| l.text_range.len()).sum();
    assert_eq!(total, text.len());
    for l in &lines {
        assert!(l.extent <= 50.01 && !l.glyphs.is_empty());
    }
    // A cluster is never split: e + combining mark stays together.
    let text = "q\u{301}q\u{301}q\u{301}q\u{301}q\u{301}q\u{301}";
    let lines = layout_text(&chain, text, 16.0, &LayoutOptions::new(1.0)).unwrap();
    assert_eq!(lines.len(), 6, "one cluster per line at a 1 px measure");
    for l in &lines {
        assert_eq!(line_text(text, l), "q\u{301}");
    }
}

#[test]
fn paragraphs_and_empty_lines() {
    let chain = chain();
    let text = "first\n\nthird paragraph\r\nfourth";
    let lines = layout_text(&chain, text, 16.0, &LayoutOptions::new(0.0)).unwrap();
    let got: Vec<(&str, usize)> = lines
        .iter()
        .map(|l| (line_text(text, l), l.paragraph))
        .collect();
    assert_eq!(
        got,
        [("first", 0), ("", 1), ("third paragraph", 2), ("fourth", 3)]
    );
    assert!(lines.iter().all(|l| l.forced_break));
    // Clusters index the whole text.
    for l in &lines {
        for g in &l.glyphs {
            assert!(l.text_range.contains(&(g.cluster as usize)));
        }
    }
    assert!(
        layout_text(&chain, "", 16.0, &LayoutOptions::new(100.0))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn alignment_and_justification() {
    let chain = chain();
    let max = 300.0;
    let lay = |align| {
        layout_text(
            &chain,
            PARAGRAPH,
            16.0,
            &LayoutOptions::new(max).with_align(align),
        )
        .unwrap()
    };
    let start = lay(Align::Start);
    let end = lay(Align::End);
    let center = lay(Align::Center);
    let justified = lay(Align::Justify);
    assert_eq!(start.len(), justified.len());
    for i in 0..start.len() {
        assert_eq!(start[i].offset, 0.0);
        assert!((end[i].offset - (max - end[i].extent)).abs() < 1e-3);
        assert!((center[i].offset - (max - center[i].extent) / 2.0).abs() < 1e-3);
        assert_eq!(start[i].text_range, justified[i].text_range);
    }
    assert_eq!(lay(Align::Left)[0].offset, 0.0);
    assert!(lay(Align::Right)[0].offset > 0.0);
    // Justified: every line but the last spans the measure exactly,
    // by wider spaces only.
    let last = justified.len() - 1;
    for (i, line) in justified.iter().enumerate() {
        if i == last {
            assert!(
                (line.extent - start[i].extent).abs() < 1e-3,
                "last line is not stretched"
            );
            continue;
        }
        assert!(
            (advance(line) - max).abs() < 0.01,
            "line {i}: {}",
            advance(line)
        );
        assert!((line.extent - max).abs() < 0.01);
        for (a, b) in line.glyphs.iter().zip(&start[i].glyphs) {
            assert_eq!(a.glyph_id, b.glyph_id);
            let is_space = PARAGRAPH.as_bytes()[a.cluster as usize] == b' ';
            if is_space {
                assert!(a.x_advance > b.x_advance);
            } else {
                assert_eq!(a.x_advance, b.x_advance);
            }
        }
    }
}

#[test]
fn right_to_left_paragraphs() {
    let chain = chain();
    let hebrew =
        "\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e2}\u{5d5}\u{5dc}\u{5dd} \u{5e9}\u{5dc}\u{5d5}\u{5dd}";
    let text = format!("Hello world\n{hebrew}");
    let lines = layout_text(&chain, &text, 16.0, &LayoutOptions::new(400.0)).unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!((lines[0].base_level, lines[1].base_level), (0, 1));
    // Start alignment: flush left, then flush right.
    assert_eq!(lines[0].offset, 0.0);
    assert!((lines[1].offset - (400.0 - lines[1].extent)).abs() < 1e-3);
    // The right-to-left line is in visual order: clusters descend.
    let clusters: Vec<u32> = lines[1].glyphs.iter().map(|g| g.cluster).collect();
    assert!(clusters.windows(2).all(|w| w[0] > w[1]), "{clusters:?}");
    // Wrapped, the first line holds the first words of the text.
    let narrow = layout_text(&chain, hebrew, 16.0, &LayoutOptions::new(90.0)).unwrap();
    assert!(narrow.len() >= 2);
    assert_eq!(narrow[0].text_range.start, 0);
    // A forced direction overrides the paragraph's own.
    let forced = layout_text(
        &chain,
        "abc",
        16.0,
        &LayoutOptions::new(200.0).with_base_level(Some(1)),
    )
    .unwrap();
    assert_eq!(forced[0].base_level, 1);
    assert!(forced[0].offset > 0.0);
}

#[test]
fn tabs_advance_to_stops() {
    let chain = chain();
    let space = chain.shape(" ", 16.0).unwrap()[0];
    let pen_after = |line: &LaidOutLine, byte: usize| -> f32 {
        let mut pen = 0.0;
        for g in &line.glyphs {
            pen += g.x_advance;
            if g.cluster as usize == byte {
                return pen;
            }
        }
        panic!("no glyph for byte {byte}");
    };
    // Default stops: every eight space widths.
    let text = "a\tbb\tc";
    let lines = layout_text(&chain, text, 16.0, &LayoutOptions::new(0.0)).unwrap();
    let interval = space.x_advance * 8.0;
    assert!((pen_after(&lines[0], 1) - interval).abs() < 0.01);
    assert!((pen_after(&lines[0], 4) - 2.0 * interval).abs() < 0.01);
    // A tab is drawn with the space glyph, not `.notdef`.
    let tab = lines[0].glyphs.iter().find(|g| g.cluster == 1).unwrap();
    assert_eq!(tab.glyph_id, space.glyph_id);
    // Explicit stops, then the interval.
    let opts = LayoutOptions::new(0.0)
        .with_tab_stops(vec![100.0, 30.0])
        .with_tab_interval(50.0);
    let lines = layout_text(&chain, "a\tb\tc\td", 16.0, &opts).unwrap();
    assert!((pen_after(&lines[0], 1) - 30.0).abs() < 0.01);
    assert!((pen_after(&lines[0], 3) - 100.0).abs() < 0.01);
    assert!((pen_after(&lines[0], 5) - 150.0).abs() < 0.01);
    // Two tabs in a row take two stops.
    let lines = layout_text(&chain, "\t\tx", 16.0, &opts).unwrap();
    assert!((pen_after(&lines[0], 1) - 100.0).abs() < 0.01);
}

#[test]
fn vertical_lines_measure_downwards() {
    let chain = chain();
    let opts =
        LayoutOptions::new(5000.0).with_shape_options(ShapeOptions::new().with_vertical(true));
    let text = "ABCDEFGH IJKLMNOP";
    let lines = layout_text(&chain, text, 2048.0, &opts).unwrap();
    assert!(lines.len() >= 2, "columns break on height");
    for l in &lines {
        let height: f32 = l.glyphs.iter().map(|g| g.y_advance).sum();
        assert!((height - l.extent).abs() < 0.01);
        assert!(l.extent <= 5000.01);
        assert!(l.glyphs.iter().all(|g| g.x_advance == 0.0));
    }
}

#[test]
fn a_large_document_lays_out_in_linear_time() {
    // 200 000 characters in one paragraph: quadratic filling would
    // take minutes even in release; this finishes in well under a
    // second there and in a few seconds unoptimised.
    let chain = chain();
    let mut text = String::new();
    while text.len() < 200_000 {
        text.push_str(PARAGRAPH);
        text.push(' ');
    }
    let started = std::time::Instant::now();
    let lines = layout_text(&chain, &text, 16.0, &LayoutOptions::new(480.0)).unwrap();
    assert!(lines.len() > 1000);
    assert_eq!(lines.last().unwrap().text_range.end, text.len());
    assert!(
        started.elapsed().as_secs() < 60,
        "took {:?}",
        started.elapsed()
    );
}
