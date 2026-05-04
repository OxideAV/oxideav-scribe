//! Pure-Rust font shaper + layout for the
//! [oxideav](https://github.com/OxideAV) framework.
//!
//! Scribe is a **vector-only shaper**: parse TTF / OTF tables → emit
//! positioned vector glyphs as [`oxideav_core::Node`]s. All pixel
//! work — outline flattening, scanline anti-aliasing, alpha
//! compositing — happens downstream in
//! [`oxideav-raster`](https://github.com/OxideAV/oxideav-raster).
//!
//! Scope:
//! - **Shaper** — `cmap` + GSUB type 4 (ligatures) + GPOS type 2
//!   (pair kerning) + mark-to-base / mark-to-mark, enough for Latin /
//!   Cyrillic / Greek / basic CJK / Vietnamese / polytonic Greek.
//! - **`Face::glyph_path` / `glyph_node`** — TrueType + OTF (CFF)
//!   outlines as `oxideav_core::Path`; CBDT/sbix colour bitmaps as
//!   `Node::Image` carrying a `VideoFrame`.
//! - **`Shaper::shape_to_paths`** — vector text API: positioned
//!   `(face_idx, Node, Transform2D)` triples ready to compose into a
//!   `VectorFrame`. Each glyph is wrapped in a cache-keyed `Group` so
//!   the downstream rasterizer's bitmap cache reuses the same memoised
//!   glyph across renders.
//! - **Face chain** — multi-face fallback (primary → fallback chain),
//!   per-codepoint resolution.
//! - **Layout** — line measurement + word-wrap (no bidi).
//!
//! See `README.md` for a tour and the deferral list.

#![deny(missing_debug_implementations)]
#![warn(rust_2018_idioms)]

pub mod color;
pub mod color_glyph;
pub mod face;
pub mod face_chain;
pub mod layout;
pub mod shaper;
pub mod style;

pub use color::{Rgba, TRANSPARENT, WHITE};
pub use color_glyph::ColorGlyphBitmap;
pub use face::{Face, FaceKind};
pub use face_chain::FaceChain;
pub use layout::{run_width, wrap_lines};
pub use shaper::{PositionedGlyph, Shaper};
pub use style::{
    synthetic_italic_shear, Style, DEFAULT_SYNTHETIC_ITALIC_DEG, ITALIC_ANGLE_EPSILON_DEG,
};

/// Errors emitted by the scribe pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The underlying TTF parser rejected the bytes.
    Ttf(oxideav_ttf::Error),
    /// The underlying OTF (CFF) parser rejected the bytes.
    Otf(oxideav_otf::Error),
    /// `size_px` was non-positive (negative or NaN).
    InvalidSize,
    /// A `with_font` / `with_otf_font` call was made on a face of
    /// the wrong flavour.
    WrongFaceKind {
        expected: FaceKind,
        actual: FaceKind,
    },
}

impl From<oxideav_ttf::Error> for Error {
    fn from(e: oxideav_ttf::Error) -> Self {
        Self::Ttf(e)
    }
}

impl From<oxideav_otf::Error> for Error {
    fn from(e: oxideav_otf::Error) -> Self {
        Self::Otf(e)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ttf(e) => write!(f, "ttf error: {e}"),
            Self::Otf(e) => write!(f, "otf error: {e}"),
            Self::InvalidSize => f.write_str("non-positive font size"),
            Self::WrongFaceKind { expected, actual } => {
                write!(f, "wrong face kind: expected {expected:?}, got {actual:?}")
            }
        }
    }
}

impl std::error::Error for Error {}
