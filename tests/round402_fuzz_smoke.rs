//! Round 402 — deterministic generative fuzz-smoke harness for the
//! shaping / layout entry points.
//!
//! Two seeded loops, both fully reproducible (a fixed-seed splitmix64
//! PRNG, no external crate, no `OXIDEAV_NETWORK_TESTS` gate):
//!
//! 1. **Font-byte mutation** — flip a handful of bytes in a valid DejaVu
//!    Sans font and feed the result to `Face::from_ttf_bytes`. Most
//!    mutations fail the sfnt structural checks (→ `Err`), but the ones
//!    that still parse are then shaped; nothing may panic.
//! 2. **Text mutation** — feed randomly-generated Unicode scalar strings
//!    (weighted toward the awkward ranges: combining marks, bidi
//!    controls, format characters, surrog.-adjacent BMP, astral) to a
//!    pristine face and the full layout pipeline.
//!
//! The harness is a *smoke* test: it asserts the never-panic contract
//! and a few structural invariants, not glyph-exact output. A failing
//! seed prints enough to reproduce.

use oxideav_scribe::{layout, Face, FaceChain};

const DEJAVU_BYTES: &[u8] = include_bytes!("fixtures/DejaVuSans.ttf");

/// Deterministic splitmix64 — a tiny, self-contained PRNG so a failing
/// run reproduces bit-for-bit from its seed.
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
    /// A Unicode scalar value weighted toward shaping-awkward ranges.
    fn scalar(&mut self) -> char {
        let cp = match self.below(10) {
            0..=2 => 0x0300 + self.below(0x70) as u32, // combining marks
            3 => 0x2000 + self.below(0x70) as u32,     // spaces / bidi controls
            4 => 0x0600 + self.below(0xFF) as u32,     // Arabic
            5 => 0x0900 + self.below(0xFF) as u32,     // Devanagari
            6 => 0xFE00 + self.below(0x20) as u32,     // variation selectors
            7 => 0x1_0000 + self.below(0x1_0000) as u32, // astral (emoji etc.)
            _ => 0x20 + self.below(0x5F) as u32,       // ASCII
        };
        char::from_u32(cp).unwrap_or('\u{FFFD}')
    }
}

// ---------- font-byte mutation ------------------------------------------

#[test]
fn mutated_font_bytes_never_panic() {
    let mut rng = Rng(0x0FF1CE_C0FFEE);
    let sample_texts = ["Hi", "fi لا हि", "A\u{0301}\u{0301}B", "😀x"];
    let mut parsed_ok = 0u32;

    for iter in 0..500u32 {
        let mut bytes = DEJAVU_BYTES.to_vec();
        // 1..=6 single-byte flips at random offsets.
        let flips = 1 + rng.below(6);
        for _ in 0..flips {
            let off = rng.below(bytes.len() as u64) as usize;
            bytes[off] ^= (rng.below(255) + 1) as u8;
        }
        match Face::from_ttf_bytes(bytes) {
            Ok(face) => {
                parsed_ok += 1;
                // A parseable-but-corrupt font must still shape without panic.
                for t in &sample_texts {
                    let gids = face.shape_text(t, &[*b"liga", *b"ccmp"]);
                    // Output length is bounded by a sane multiple of input.
                    assert!(
                        gids.len() <= t.chars().count() * 8 + 16,
                        "seed iter {iter}: implausible glyph explosion {} for {t:?}",
                        gids.len()
                    );
                    let _ = face.position_text_itemized(t, 14.0, &[]);
                }
            }
            Err(_) => { /* structural rejection — the common, correct path */ }
        }
    }
    // Sanity: single-byte flips of a real font *sometimes* still parse,
    // so the shaping arm is actually exercised (not a vacuous loop).
    assert!(
        parsed_ok > 0,
        "no mutated font ever parsed — shaping arm never ran"
    );
}

// ---------- text mutation -----------------------------------------------

#[test]
fn mutated_text_never_panics_the_pipeline() {
    let face = Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("DejaVu parses");
    let chain = FaceChain::new(Face::from_ttf_bytes(DEJAVU_BYTES.to_vec()).expect("parses"));
    let mut rng = Rng(0xDEAD_BEEF_1234);

    for _ in 0..400u32 {
        let len = rng.below(20) as usize;
        let text: String = (0..len).map(|_| rng.scalar()).collect();

        let _ = face.shape_text(&text, &[]);
        let _ = face.shape_text_itemized(&text, &[*b"liga"]);
        let _ = face.position_text_itemized(&text, 13.0, &[]);

        // Bidi reordering must always yield a valid permutation.
        let vl = layout::reorder_line_visual(&text, None);
        assert_eq!(vl.len(), text.chars().count());

        let _ = layout::shape_paragraphs(&chain, &text, 16.0, 120.0, None);
    }
}
