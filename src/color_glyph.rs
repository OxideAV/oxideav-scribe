//! Color-glyph rasterizer — bridges `oxideav_ttf::ColorBitmap` (raw
//! CBDT PNG bytes + per-glyph metrics) to a [`crate::RgbaBitmap`].
//!
//! Round-5 scope: CBDT/CBLC color bitmap glyphs (Google's embedded-PNG
//! emoji format used by Noto Color Emoji and most Android emoji
//! fonts). The two other color-glyph table families — Microsoft's
//! COLR (layered vectors) + Apple's sbix (PNG/JPEG strikes) — are
//! deferred to future rounds.
//!
//! ## Pipeline
//!
//! 1. The shaper / face-chain produces a `PositionedGlyph`.
//! 2. The composer asks the face for the colour bitmap at the requested
//!    `size_px` via [`Face::raster_color_glyph`]. That entry point
//!    walks CBLC → CBDT and hands the raw PNG byte stream to
//!    `oxideav_png::decode_png_to_frame`.
//! 3. The decoded `VideoFrame` is unwrapped to an [`RgbaBitmap`]
//!    (always Rgba8 because Noto Color Emoji + every other CBDT font
//!    we've seen ships colour type 6 / 8-bit RGBA PNGs). Other PNG
//!    pixel formats are converted on the fly: Rgb24 → opaque RGBA,
//!    Ya8 → grayscale-as-RGB-with-alpha, Pal8 + tRNS → RGBA via the
//!    palette + per-entry alpha. (Decode-time PNG colour conversion
//!    is the consumer crate's responsibility per the upstream
//!    `oxideav-png` API contract.)
//!
//! ## Scaling
//!
//! CBDT entries carry a per-strike `ppem` (typically 109 or 136 for
//! Noto Color Emoji). The face picks the closest strike via
//! [`oxideav_ttf::Font::glyph_color_bitmap`]; if `size_px` doesn't
//! match the strike exactly, the resulting RGBA is the strike's
//! native dimensions and the caller is expected to scale during
//! composition. Round-5 doesn't perform that scale itself — the
//! emoji test verifies "the right strike was selected and the bitmap
//! decoded successfully", not "the bitmap was rescaled to size_px".
//!
//! No third-party PNG / image crate is used per workspace policy;
//! `oxideav-png` (a sibling) is the sole PNG dependency.

use crate::compose::RgbaBitmap;
use crate::face::Face;
use crate::Error;

use oxideav_core::VideoFrame;

/// Result of decoding one CBDT entry — the rasterised RGBA bitmap plus
/// the per-glyph metrics needed for placement.
#[derive(Debug, Clone)]
pub struct ColorGlyphBitmap {
    /// Decoded RGBA8 bitmap (straight alpha; same convention as the
    /// rest of the scribe pipeline).
    pub bitmap: RgbaBitmap,
    /// Distance in pixels from the horizontal pen origin to the LEFT
    /// edge of the bitmap (positive = bitmap starts to the right of
    /// the pen).
    pub bearing_x: i32,
    /// Distance in pixels from the horizontal pen origin to the TOP
    /// edge of the bitmap (positive = bitmap top is above the pen).
    /// In raster-Y-down coordinates the placement Y is `pen_y -
    /// bearing_y`.
    pub bearing_y: i32,
    /// Horizontal advance the strike author chose for this glyph,
    /// in pixels at the strike's native ppem.
    pub advance: u32,
    /// The CBDT strike's native pixels-per-em. Callers comparing to
    /// their requested `size_px` can compute a scale factor as
    /// `size_px / ppem as f32`.
    pub ppem: u8,
}

impl Face {
    /// `true` when this face ships CBDT/CBLC tables. Wraps
    /// [`oxideav_ttf::Font::has_color_bitmaps`].
    pub fn has_color_bitmaps(&self) -> bool {
        // OTF (CFF) faces don't ship CBDT in any font we've seen;
        // short-circuit to avoid the re-parse.
        match self.kind() {
            crate::FaceKind::Otf => false,
            crate::FaceKind::Ttf => self.with_font(|f| f.has_color_bitmaps()).unwrap_or(false),
        }
    }

    /// All `(ppem_x, ppem_y)` strikes the face's CBDT/CBLC tables ship.
    /// Empty when the face has no colour bitmaps.
    pub fn color_strike_sizes(&self) -> Vec<(u8, u8)> {
        match self.kind() {
            crate::FaceKind::Otf => Vec::new(),
            crate::FaceKind::Ttf => self
                .with_font(|f| f.color_strike_sizes())
                .unwrap_or_default(),
        }
    }

    /// Rasterise the colour bitmap for `glyph_id` at the strike whose
    /// `ppem_y` is closest to `size_px.round()`.
    ///
    /// Returns `Ok(None)` if the face has no CBDT/CBLC, or no strike
    /// covers the glyph, or the per-glyph CBDT entry is in a format we
    /// don't decode (anything other than 17/18/19 — the three
    /// PNG-payload formats).
    ///
    /// Returns `Err(Error::InvalidSize)` if `size_px` is non-positive
    /// or NaN, mirroring the rest of the rasterizer entry points.
    pub fn raster_color_glyph(
        &self,
        glyph_id: u16,
        size_px: f32,
    ) -> Result<Option<ColorGlyphBitmap>, Error> {
        if size_px <= 0.0 || !size_px.is_finite() {
            return Err(Error::InvalidSize);
        }
        if self.kind() != crate::FaceKind::Ttf {
            return Ok(None);
        }
        let target_ppem = size_px.round().clamp(1.0, 255.0) as u8;
        let bitmap_descriptor = self.with_font(|f| {
            f.glyph_color_bitmap(glyph_id, target_ppem).map(|cb| {
                (
                    cb.png_bytes.to_vec(),
                    cb.width,
                    cb.height,
                    cb.bearing_x,
                    cb.bearing_y,
                    cb.advance,
                    cb.ppem,
                )
            })
        })?;
        let (png_bytes, _meta_w, _meta_h, bx, by, adv, ppem) = match bitmap_descriptor {
            Some(t) => t,
            None => return Ok(None),
        };
        let frame = match oxideav_png::decode_png_to_frame(&png_bytes, None) {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };
        // Recover the true PNG image dimensions from the IHDR chunk
        // (frame metadata doesn't carry width/height — see VideoFrame
        // doc-comment in oxideav-core). CBDT metrics CAN round-down
        // vs the PNG's true pixel grid (legal per spec — metrics are
        // the layout box, the PNG can be larger and is drawn into
        // that box).
        let (png_w, png_h) = match read_png_dimensions(&png_bytes) {
            Some(d) => d,
            None => return Ok(None),
        };
        let bitmap = videoframe_to_rgba(&frame, png_w, png_h);
        Ok(Some(ColorGlyphBitmap {
            bitmap,
            bearing_x: bx as i32,
            bearing_y: by as i32,
            advance: adv as u32,
            ppem,
        }))
    }
}

/// Convert a VideoFrame from `oxideav-png` into a straight-alpha RGBA8
/// [`RgbaBitmap`] given the PNG's true pixel dimensions (recovered
/// from the IHDR chunk by [`read_png_dimensions`]).
///
/// Handles the four PNG output flavours we'll ever see from a CBDT
/// entry — derived from `plane.stride / width`:
///
/// - 4 B/px → `Rgba` (common case): copy through unchanged.
/// - 3 B/px → `Rgb24` (some glyphs ship without alpha): pad to opaque.
/// - 2 B/px → `Ya8` (grayscale + alpha): splat luma into RGB.
/// - 1 B/px → `Gray8` / `Pal8` index (atypical for CBDT): splat luma,
///   opaque alpha. We don't apply palette lookup; the result is a
///   grayscale view of the index byte. CBDT-PNG never ships Pal8
///   in practice (Noto Color Emoji is colour type 6).
/// - any other ratio → empty bitmap (the composer skips empty glyphs).
fn videoframe_to_rgba(frame: &VideoFrame, width: u32, height: u32) -> RgbaBitmap {
    if frame.planes.is_empty() || width == 0 || height == 0 {
        return RgbaBitmap::default();
    }
    let plane = &frame.planes[0];
    if plane.stride == 0 || plane.data.is_empty() {
        return RgbaBitmap::default();
    }
    let w = width as usize;
    let h = height as usize;
    // bytes-per-pixel inferred from `stride / width` (oxideav-png
    // packs rows tightly with no padding).
    if plane.stride % w != 0 {
        return RgbaBitmap::default();
    }
    let bpp = plane.stride / w;
    if !(1..=4).contains(&bpp) {
        return RgbaBitmap::default();
    }
    if plane.data.len() < plane.stride * h {
        return RgbaBitmap::default();
    }
    let mut out = RgbaBitmap::new(width, height);
    for row in 0..h {
        for col in 0..w {
            let src_off = row * plane.stride + col * bpp;
            let dst_off = (row * w + col) * 4;
            match bpp {
                4 => {
                    out.data[dst_off] = plane.data[src_off];
                    out.data[dst_off + 1] = plane.data[src_off + 1];
                    out.data[dst_off + 2] = plane.data[src_off + 2];
                    out.data[dst_off + 3] = plane.data[src_off + 3];
                }
                3 => {
                    out.data[dst_off] = plane.data[src_off];
                    out.data[dst_off + 1] = plane.data[src_off + 1];
                    out.data[dst_off + 2] = plane.data[src_off + 2];
                    out.data[dst_off + 3] = 255;
                }
                2 => {
                    let y = plane.data[src_off];
                    let a = plane.data[src_off + 1];
                    out.data[dst_off] = y;
                    out.data[dst_off + 1] = y;
                    out.data[dst_off + 2] = y;
                    out.data[dst_off + 3] = a;
                }
                1 => {
                    let y = plane.data[src_off];
                    out.data[dst_off] = y;
                    out.data[dst_off + 1] = y;
                    out.data[dst_off + 2] = y;
                    out.data[dst_off + 3] = 255;
                }
                _ => unreachable!(),
            }
        }
    }
    out
}

/// Read `(width, height)` from the IHDR chunk of a PNG byte stream.
/// Returns `None` for streams that don't start with the standard 8-byte
/// PNG signature followed by a 13-byte IHDR chunk in the canonical
/// position (every spec-conformant PNG does — IHDR is required to be
/// the first chunk).
///
/// We avoid decoding the full IHDR via `oxideav_png::Ihdr::parse`
/// because that's an internal API and pulling more of `oxideav-png` in
/// here would tighten the coupling unnecessarily; the four bytes at
/// known offsets are enough.
fn read_png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    // PNG signature (8) + chunk length (4) + chunk type (4) = 16.
    // IHDR data starts at offset 16 with width:u32 + height:u32.
    if bytes.len() < 24 {
        return None;
    }
    if &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    if &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::{VideoFrame, VideoPlane};

    /// 2×2 RGBA frame round-trip — the rgba8 happy path.
    #[test]
    fn videoframe_rgba8_to_bitmap() {
        let frame = VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: 8, // 2 px * 4 B
                data: vec![
                    255, 0, 0, 255, // (0,0) red
                    0, 255, 0, 128, // (1,0) green half-alpha
                    0, 0, 255, 64, // (0,1) blue quarter-alpha
                    255, 255, 0, 255, // (1,1) yellow opaque
                ],
            }],
        };
        let bm = videoframe_to_rgba(&frame, 2, 2);
        assert_eq!(bm.width, 2);
        assert_eq!(bm.height, 2);
        assert_eq!(bm.get(0, 0), [255, 0, 0, 255]);
        assert_eq!(bm.get(1, 0), [0, 255, 0, 128]);
        assert_eq!(bm.get(0, 1), [0, 0, 255, 64]);
        assert_eq!(bm.get(1, 1), [255, 255, 0, 255]);
    }

    #[test]
    fn videoframe_rgb24_to_bitmap_fills_opaque_alpha() {
        let frame = VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: 6, // 2 px * 3 B
                data: vec![
                    255, 0, 0, // red
                    0, 255, 0, // green
                ],
            }],
        };
        let bm = videoframe_to_rgba(&frame, 2, 1);
        assert_eq!(bm.width, 2);
        assert_eq!(bm.height, 1);
        assert_eq!(bm.get(0, 0), [255, 0, 0, 255]);
        assert_eq!(bm.get(1, 0), [0, 255, 0, 255]);
    }

    #[test]
    fn empty_videoframe_returns_empty_bitmap() {
        let frame = VideoFrame {
            pts: None,
            planes: vec![],
        };
        let bm = videoframe_to_rgba(&frame, 0, 0);
        assert!(bm.is_empty());
    }

    /// Real PNG signature + IHDR — verify the IHDR width/height
    /// extraction without pulling oxideav-png into the test.
    #[test]
    fn read_png_dimensions_extracts_ihdr() {
        let mut buf: Vec<u8> = Vec::new();
        // Signature.
        buf.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        // IHDR length (13).
        buf.extend_from_slice(&13u32.to_be_bytes());
        // IHDR type.
        buf.extend_from_slice(b"IHDR");
        // 96 wide x 109 tall, 8-bit, ct=6, etc.
        buf.extend_from_slice(&96u32.to_be_bytes());
        buf.extend_from_slice(&109u32.to_be_bytes());
        buf.extend_from_slice(&[8, 6, 0, 0, 0]);
        // CRC stub (4 B); ignored.
        buf.extend_from_slice(&[0; 4]);
        let dim = read_png_dimensions(&buf).expect("ihdr");
        assert_eq!(dim, (96, 109));

        // Wrong signature → None.
        let mut bad = buf.clone();
        bad[0] = 0;
        assert!(read_png_dimensions(&bad).is_none());

        // Truncated → None.
        assert!(read_png_dimensions(&buf[..16]).is_none());
    }
}
