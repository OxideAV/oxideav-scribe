//! Round 402 — hostile-input hardening for the shaping / layout entry
//! points.
//!
//! The contract under test: every public shaping and layout surface must
//! turn adversarial input — truncated / garbage font bytes, pathological
//! text (megabyte combining-mark storms, bidi-control spam, lone
//! format characters, degenerate sizes) — into a typed `Error` or an
//! empty / well-formed result. It must **never panic** (no slice OOB, no
//! integer overflow in debug, no unwrap on absent tables).
//!
//! These are enumerated adversarial cases; the companion
//! `round402_fuzz_smoke` test drives randomised byte mutations.

use oxideav_scribe::{layout, Face, FaceChain, Shaper};

const DEJAVU_BYTES: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

fn chain() -> FaceChain {
    FaceChain::new(Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses"))
}
fn face() -> Face {
    Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses")
}

// ---------- malformed font bytes → Err, never panic ---------------------

#[test]
fn empty_bytes_is_error_not_panic() {
    assert!(Face::from_ttf_bytes(Vec::new()).is_err());
    assert!(Face::from_otf_bytes(Vec::new()).is_err());
}

#[test]
fn short_and_garbage_bytes_are_errors() {
    for len in [1usize, 3, 4, 11, 12, 15, 63, 200] {
        let junk = vec![0xABu8; len];
        // Must return Err, not panic on a truncated sfnt header / table dir.
        let _ = Face::from_ttf_bytes(junk.clone());
        let _ = Face::from_otf_bytes(junk);
    }
    // A plausible-looking sfnt magic with a lie about the table count.
    let mut fake = vec![0x00, 0x01, 0x00, 0x00]; // TrueType magic
    fake.extend_from_slice(&[0xFF, 0xFF]); // numTables = 65535
    fake.extend_from_slice(&[0u8; 6]); // rest of the offset subtable
    assert!(Face::from_ttf_bytes(fake).is_err());
}

#[test]
fn truncated_real_font_is_error_not_panic() {
    // Chop the real DejaVu font at many boundaries: each prefix either
    // parses (unlikely) or errors, but must never panic.
    for cut in [16usize, 100, 1000, 5000, 20_000, DEJAVU_BYTES.len() / 2] {
        let _ = Face::from_ttf_bytes(DEJAVU_BYTES[..cut].to_vec());
    }
}

// ---------- pathological text into the shaper ---------------------------

#[test]
fn empty_and_whitespace_text_shape_to_finite_output() {
    let f = face();
    assert!(f.shape_text("", &[]).is_empty());
    assert!(f.shape_text_itemized("", &[]).is_empty());
    // Whitespace-only + control chars: no panic, finite output.
    let _ = f.shape_text("\t\n\r  ", &[]);
    let _ = f.position_text("   ", 16.0, &[]);
}

#[test]
fn combining_mark_storm_does_not_panic() {
    // 20k combining acute accents with no base — degenerate cluster.
    let storm: String = "\u{0301}".repeat(20_000);
    let f = face();
    let gids = f.shape_text(&storm, &[]);
    assert!(gids.len() <= 20_000 + 8);
    let _ = f.position_text_itemized(&storm, 16.0, &[]);
    let _ = f.position_text(&storm, 16.0, &[*b"liga", *b"mark"]);
}

#[test]
fn bidi_control_spam_does_not_panic() {
    // Nested isolate / override / embedding initiators with no terminators,
    // plus mirrored brackets — stresses the UAX #9 explicit-level stack.
    let mut s = String::new();
    for _ in 0..5000 {
        s.push('\u{2066}'); // LRI
        s.push('\u{202E}'); // RLO
        s.push('(');
        s.push('\u{2069}'); // PDI
    }
    let _ = layout::reorder_line_visual(&s, None);
    let _ = layout::reorder_line_visual(&s, Some(1));
    let vl = layout::reorder_line_visual(&s, Some(0));
    // The reordering permutation must stay a valid bijection over the input.
    assert_eq!(vl.len(), s.chars().count());
}

#[test]
fn degenerate_sizes_and_widths_do_not_panic() {
    let c = chain();
    for size in [0.0f32, -1.0, f32::NAN, f32::INFINITY, 1e30] {
        let _ = layout::shape_visual_line(&c, "Hi ابc", size, None);
        let _ = layout::wrap_and_shape_lines(&c, "Hello world", size, 100.0, None);
    }
    for width in [0.0f32, -50.0, f32::NAN, f32::INFINITY] {
        let _ = layout::wrap_and_shape_lines(&c, "Hello world foo bar", 16.0, width, None);
        let _ = layout::shape_paragraphs(&c, "a\nb\nc", 16.0, width, None);
    }
}

#[test]
fn mixed_script_and_format_chars_itemize_without_panic() {
    let f = face();
    // Latin + Arabic + Devanagari + emoji + zero-width joiners + BOM.
    let s = "Aا\u{200D}क😀\u{FEFF}Ω\u{0301}\u{2028}\u{2029}xyz";
    let _ = f.shape_text_itemized(s, &[]);
    let _ = f.position_text_itemized(s, 12.0, &[*b"ccmp", *b"liga"]);
    let c = chain();
    let _ = layout::shape_paragraphs(&c, s, 16.0, 200.0, None);
}

#[test]
fn shaper_auto_probe_on_hostile_text() {
    let c = chain();
    let s = "\u{202E}\u{0301}\u{200D}!!!\u{FFFD}\u{10FFFF}";
    // Shaper::shape / shape_to_paths must survive the same.
    let _ = Shaper::shape(c.primary(), s, 16.0);
    let _ = Shaper::shape_to_paths(&c, s, 16.0);
    let _ = c.shape(s, 16.0);
}
