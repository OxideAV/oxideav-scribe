//! Color-glyph rasterizer — bridges `oxideav_ttf::ColorBitmap` (raw
//! CBDT PNG bytes + per-glyph metrics) to a straight-alpha RGBA8
//! buffer.
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
//! match the strike exactly, [`Face::raster_color_glyph_at`] runs a
//! bilinear resample to the requested size before handing the bitmap
//! back. Once the bitmap reaches `Face::glyph_node` it gets wrapped in
//! an `oxideav_core::ImageRef` carrying a `VideoFrame` so downstream
//! `oxideav-raster` blits it through the same image-rendering path it
//! uses for any other embedded raster.
//!
//! No third-party PNG / image crate is used per workspace policy;
//! `oxideav-png` (a sibling) is the sole PNG dependency.

use crate::face::Face;
use crate::Error;

use oxideav_core::VideoFrame;

/// A grayscale-irrelevant straight-alpha RGBA8 bitmap. Stride is
/// `width * 4`. Used internally to carry the decoded colour-glyph
/// pixels through resampling before they're wrapped in a
/// `VideoFrame` for the outer `Node::Image`.
#[derive(Debug, Clone, Default)]
pub struct RgbaBitmap {
    /// Bitmap width in pixels.
    pub width: u32,
    /// Bitmap height in pixels.
    pub height: u32,
    /// Row-major straight-alpha RGBA8 bytes (`width * height * 4`).
    pub data: Vec<u8>,
}

impl RgbaBitmap {
    /// Allocate a fully-transparent (alpha = 0) bitmap.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; (width as usize) * (height as usize) * 4],
        }
    }

    /// True if the bitmap holds zero pixels.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Read RGBA at `(x, y)`. Out-of-range reads return `[0; 4]`.
    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let off = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        [
            self.data[off],
            self.data[off + 1],
            self.data[off + 2],
            self.data[off + 3],
        ]
    }

    /// Number of pixels with non-zero alpha.
    pub fn nonzero_alpha_count(&self) -> usize {
        self.data.chunks_exact(4).filter(|p| p[3] != 0).count()
    }

    /// Bilinearly resample this bitmap to `(dst_width, dst_height)`.
    ///
    /// Used by the colour-bitmap pipeline to scale a CBDT strike
    /// (typically 109 px or 136 px ppem for Noto Color Emoji) down to
    /// the requested raster size. Edge sampling clamps at the source
    /// borders so we never read outside the bitmap. Interpolation is
    /// performed in **straight-alpha space** independently per channel
    /// — the simpler model that matches what FreeType's bitmap-strike
    /// scaling does. Premultiplied interpolation produces sharper
    /// alpha-edge silhouettes but requires un-premultiplying afterwards
    /// to keep downstream consumers happy; for emoji glyphs at typical
    /// body-text sizes the visual difference is imperceptible.
    ///
    /// Returns the same bitmap unchanged when `dst_width == self.width`
    /// and `dst_height == self.height` (cheap pass-through). Returns an
    /// empty bitmap when either source or destination has a zero
    /// dimension.
    pub fn resample_bilinear(&self, dst_width: u32, dst_height: u32) -> RgbaBitmap {
        if self.is_empty() || dst_width == 0 || dst_height == 0 {
            return RgbaBitmap::default();
        }
        if dst_width == self.width && dst_height == self.height {
            return self.clone();
        }
        let src_w = self.width as usize;
        let src_h = self.height as usize;
        let dw = dst_width as usize;
        let dh = dst_height as usize;
        let mut out = RgbaBitmap::new(dst_width, dst_height);
        // Map each destination pixel centre to a source coordinate via
        // half-pixel offsets so the corner samples land on the source
        // corner pixel centres (the standard "centre-sample" mapping).
        let sx = self.width as f32 / dst_width as f32;
        let sy = self.height as f32 / dst_height as f32;
        for dy in 0..dh {
            // Source Y at the destination pixel centre.
            let src_y = (dy as f32 + 0.5) * sy - 0.5;
            let y0_f = src_y.floor();
            let fy = src_y - y0_f;
            let y0 = (y0_f as i32).clamp(0, src_h as i32 - 1) as usize;
            let y1 = (y0_f as i32 + 1).clamp(0, src_h as i32 - 1) as usize;
            for dx in 0..dw {
                let src_x = (dx as f32 + 0.5) * sx - 0.5;
                let x0_f = src_x.floor();
                let fx = src_x - x0_f;
                let x0 = (x0_f as i32).clamp(0, src_w as i32 - 1) as usize;
                let x1 = (x0_f as i32 + 1).clamp(0, src_w as i32 - 1) as usize;
                let off00 = (y0 * src_w + x0) * 4;
                let off10 = (y0 * src_w + x1) * 4;
                let off01 = (y1 * src_w + x0) * 4;
                let off11 = (y1 * src_w + x1) * 4;
                let dst_off = (dy * dw + dx) * 4;
                let w00 = (1.0 - fx) * (1.0 - fy);
                let w10 = fx * (1.0 - fy);
                let w01 = (1.0 - fx) * fy;
                let w11 = fx * fy;
                for c in 0..4 {
                    let s00 = self.data[off00 + c] as f32;
                    let s10 = self.data[off10 + c] as f32;
                    let s01 = self.data[off01 + c] as f32;
                    let s11 = self.data[off11 + c] as f32;
                    let mixed = s00 * w00 + s10 * w10 + s01 * w01 + s11 * w11;
                    out.data[dst_off + c] = mixed.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        out
    }
}

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
    /// `ppem_y` is closest to `size_px.round()`, **at the strike's
    /// native pixel dimensions**.
    ///
    /// The returned bitmap is the un-scaled CBDT strike — typically
    /// 109 px or 136 px on a side for Noto Color Emoji even when the
    /// caller asked for `size_px = 32`. Use
    /// [`Face::raster_color_glyph_at`] when you want the bitmap
    /// pre-resampled to `size_px`.
    ///
    /// Returns `Ok(None)` if the face has no CBDT/CBLC, or no strike
    /// covers the glyph, or the per-glyph CBDT entry is in a format we
    /// don't decode (anything other than 17/18/19 — the three
    /// PNG-payload formats).
    ///
    /// Returns `Err(Error::InvalidSize)` if `size_px` is non-positive
    /// or NaN, mirroring the rest of the scribe entry points.
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
        // Noto Color Emoji ships PLTE-encoded (colour type 3) PNGs —
        // smaller payload than direct RGBA. `oxideav_png::decode_png_to_frame`
        // hands those back as a 1-byte-per-pixel `Pal8` plane, with the
        // PLTE/tRNS chunks NOT exposed through the `VideoFrame`. We
        // re-parse those two tiny chunks here at the boundary (same
        // pattern as `read_png_dimensions` already does for IHDR) and
        // splice them into the conversion. Direct RGBA strikes (Apple
        // Color Emoji, EmojiOne) skip the palette path entirely —
        // `read_png_palette` returns `None` and we fall through to the
        // existing colour-type sniff in `videoframe_to_rgba`.
        let palette = read_png_palette(&png_bytes);
        let bitmap = videoframe_to_rgba(&frame, png_w, png_h, palette.as_ref());
        Ok(Some(ColorGlyphBitmap {
            bitmap,
            bearing_x: bx as i32,
            bearing_y: by as i32,
            advance: adv as u32,
            ppem,
        }))
    }

    /// Rasterise the colour bitmap for `glyph_id`, **bilinearly
    /// resampled** to the requested `size_px`.
    ///
    /// Walks CBLC → CBDT to find the closest strike, decodes the PNG to
    /// straight-alpha RGBA8 (via [`Face::raster_color_glyph`]), then
    /// runs [`RgbaBitmap::resample_bilinear`] to scale the bitmap to
    /// the dimensions matching `size_px` at the strike's aspect ratio.
    /// The returned [`ColorGlyphBitmap::bearing_x`] / `bearing_y` /
    /// `advance` are also pre-scaled by `size_px / ppem` (rounded),
    /// and `ppem` reports the requested raster size (not the strike's
    /// native ppem) so the caller can use the metrics directly without
    /// a second-stage scale.
    ///
    /// Returns the same `Ok(None)` / `Err(Error::InvalidSize)` cases as
    /// [`Face::raster_color_glyph`].
    pub fn raster_color_glyph_at(
        &self,
        glyph_id: u16,
        size_px: f32,
    ) -> Result<Option<ColorGlyphBitmap>, Error> {
        let native = match self.raster_color_glyph(glyph_id, size_px)? {
            Some(c) => c,
            None => return Ok(None),
        };
        if native.bitmap.is_empty() || native.ppem == 0 {
            return Ok(Some(native));
        }
        let strike_scale = size_px / native.ppem as f32;
        let new_w = (native.bitmap.width as f32 * strike_scale).round().max(1.0) as u32;
        let new_h = (native.bitmap.height as f32 * strike_scale)
            .round()
            .max(1.0) as u32;
        let resampled = native.bitmap.resample_bilinear(new_w, new_h);
        let new_bx = (native.bearing_x as f32 * strike_scale).round() as i32;
        let new_by = (native.bearing_y as f32 * strike_scale).round() as i32;
        let new_adv = (native.advance as f32 * strike_scale).round().max(0.0) as u32;
        // Clamp the reported "ppem" to a u8 (CBDT spec range) — at
        // size_px > 255 we cap rather than wrap. Callers wanting the
        // exact requested raster size should use `size_px` directly.
        let reported_ppem = size_px.round().clamp(1.0, 255.0) as u8;
        Ok(Some(ColorGlyphBitmap {
            bitmap: resampled,
            bearing_x: new_bx,
            bearing_y: new_by,
            advance: new_adv,
            ppem: reported_ppem,
        }))
    }
}

/// Convert a VideoFrame from `oxideav-png` into a straight-alpha RGBA8
/// [`RgbaBitmap`] given the PNG's true pixel dimensions (recovered
/// from the IHDR chunk by [`read_png_dimensions`]) and an optional
/// palette (recovered from PLTE + tRNS chunks by
/// [`read_png_palette`]).
///
/// Handles the four PNG output flavours we'll ever see from a CBDT
/// entry — derived from `plane.stride / width`:
///
/// - 4 B/px → `Rgba` (common case for Apple Color Emoji, EmojiOne):
///   copy through unchanged.
/// - 3 B/px → `Rgb24` (some glyphs ship without alpha): pad to opaque.
/// - 2 B/px → `Ya8` (grayscale + alpha): splat luma into RGB.
/// - 1 B/px → `Gray8` (no `palette`) OR `Pal8` (`palette` is `Some`):
///   - When `palette` is supplied, do a per-pixel palette lookup (Noto
///     Color Emoji ships colour type 3 PNGs — palette + per-entry
///     alpha via tRNS — for compactness).
///   - Otherwise splat the byte as grayscale + opaque alpha (grayscale
///     CBDT PNGs are rare but spec-legal).
/// - any other ratio → empty bitmap (the composer skips empty glyphs).
fn videoframe_to_rgba(
    frame: &VideoFrame,
    width: u32,
    height: u32,
    palette: Option<&PngPalette>,
) -> RgbaBitmap {
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
                    let idx = plane.data[src_off];
                    if let Some(p) = palette {
                        let rgba = p.lookup(idx);
                        out.data[dst_off] = rgba[0];
                        out.data[dst_off + 1] = rgba[1];
                        out.data[dst_off + 2] = rgba[2];
                        out.data[dst_off + 3] = rgba[3];
                    } else {
                        out.data[dst_off] = idx;
                        out.data[dst_off + 1] = idx;
                        out.data[dst_off + 2] = idx;
                        out.data[dst_off + 3] = 255;
                    }
                }
                _ => unreachable!(),
            }
        }
    }
    out
}

/// PNG palette parsed from the PLTE + tRNS chunks. PLTE always carries
/// 1..=256 RGB triplets; tRNS (when present, ≤ palette length) carries
/// per-entry alpha bytes for the leading entries (entries past tRNS's
/// length are opaque).
#[derive(Debug, Clone)]
struct PngPalette {
    /// Per-index RGBA. `entries.len()` matches PLTE's entry count
    /// (1..=256).
    entries: Vec<[u8; 4]>,
}

impl PngPalette {
    /// Look up an index. Out-of-range indices return transparent black
    /// (matching what FreeType + libpng do for malformed palettes).
    fn lookup(&self, idx: u8) -> [u8; 4] {
        self.entries
            .get(idx as usize)
            .copied()
            .unwrap_or([0, 0, 0, 0])
    }
}

/// Walk PNG chunks looking for `PLTE` + `tRNS`. Returns `Some(palette)`
/// when both PLTE is present (palette PNGs only) — the tRNS chunk is
/// optional (without it every entry is opaque). For non-palette PNGs
/// (no PLTE chunk in the stream) returns `None` so the caller falls
/// back to the existing colour-type heuristic in
/// [`videoframe_to_rgba`].
///
/// Chunk format per PNG §5.3: `length (u32 BE) + type (4 ASCII) +
/// data + CRC (u32 BE)`. The 8-byte signature precedes the first
/// chunk. Walking is bounds-checked against the slice length on every
/// step.
fn read_png_palette(bytes: &[u8]) -> Option<PngPalette> {
    if bytes.len() < 8 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let mut off = 8usize;
    let mut plte: Option<&[u8]> = None;
    let mut trns: Option<&[u8]> = None;
    while off + 12 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
            as usize;
        let chunk_end = off.checked_add(8).and_then(|x| x.checked_add(len))?;
        // CRC is 4 bytes after data; total chunk footprint = 12 + len.
        let total_end = chunk_end.checked_add(4)?;
        if total_end > bytes.len() {
            break;
        }
        let chunk_type = &bytes[off + 4..off + 8];
        let chunk_data = &bytes[off + 8..chunk_end];
        match chunk_type {
            b"PLTE" => plte = Some(chunk_data),
            b"tRNS" => trns = Some(chunk_data),
            b"IDAT" => {
                // IDAT comes after PLTE/tRNS per PNG ordering; once we
                // hit it we know we won't find any more colour-type-3
                // metadata. Bail early to keep the walk bounded.
                break;
            }
            b"IEND" => break,
            _ => {}
        }
        off = total_end;
    }
    let plte = plte?;
    if plte.is_empty() || plte.len() % 3 != 0 || plte.len() > 256 * 3 {
        return None;
    }
    let n = plte.len() / 3;
    let mut entries: Vec<[u8; 4]> = Vec::with_capacity(n);
    for i in 0..n {
        entries.push([plte[i * 3], plte[i * 3 + 1], plte[i * 3 + 2], 255]);
    }
    if let Some(t) = trns {
        // Per spec the tRNS chunk for colour type 3 carries up to N
        // alpha bytes (one per palette entry); entries past tRNS's
        // length stay opaque.
        let m = t.len().min(n);
        for (i, &alpha) in t.iter().take(m).enumerate() {
            entries[i][3] = alpha;
        }
    }
    Some(PngPalette { entries })
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
        let bm = videoframe_to_rgba(&frame, 2, 2, None);
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
        let bm = videoframe_to_rgba(&frame, 2, 1, None);
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
        let bm = videoframe_to_rgba(&frame, 0, 0, None);
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

    /// Build a synthetic PNG with PLTE + tRNS, walk it via
    /// `read_png_palette`. Verifies (a) the chunk walker traverses past
    /// IHDR + PLTE + tRNS to the entries, (b) the palette/alpha pairing
    /// is correct, (c) the bail at IDAT works, (d) palette-less PNGs
    /// return `None`.
    #[test]
    fn read_png_palette_extracts_plte_and_trns() {
        // Construct: signature + IHDR + PLTE + tRNS + IDAT + IEND.
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        // IHDR (length 13)
        buf.extend_from_slice(&13u32.to_be_bytes());
        buf.extend_from_slice(b"IHDR");
        buf.extend_from_slice(&8u32.to_be_bytes()); // width
        buf.extend_from_slice(&8u32.to_be_bytes()); // height
        buf.extend_from_slice(&[8, 3, 0, 0, 0]); // depth, ct=3 (palette)
        buf.extend_from_slice(&[0u8; 4]); // CRC stub
                                          // PLTE: 3 entries (red, green, blue)
        buf.extend_from_slice(&9u32.to_be_bytes()); // length 9
        buf.extend_from_slice(b"PLTE");
        buf.extend_from_slice(&[
            255, 0, 0, // red
            0, 255, 0, // green
            0, 0, 255, // blue
        ]);
        buf.extend_from_slice(&[0u8; 4]); // CRC stub
                                          // tRNS: 2 entries (red opaque, green half-alpha) — the third
                                          // palette entry stays opaque.
        buf.extend_from_slice(&2u32.to_be_bytes()); // length 2
        buf.extend_from_slice(b"tRNS");
        buf.extend_from_slice(&[255, 128]);
        buf.extend_from_slice(&[0u8; 4]); // CRC stub
                                          // IDAT placeholder so the walker bails before IEND.
        buf.extend_from_slice(&0u32.to_be_bytes());
        buf.extend_from_slice(b"IDAT");
        buf.extend_from_slice(&[0u8; 4]); // CRC stub

        let pal = read_png_palette(&buf).expect("palette");
        assert_eq!(pal.entries.len(), 3);
        assert_eq!(pal.lookup(0), [255, 0, 0, 255]);
        assert_eq!(pal.lookup(1), [0, 255, 0, 128]);
        assert_eq!(pal.lookup(2), [0, 0, 255, 255]);
        // Out-of-range index → transparent black.
        assert_eq!(pal.lookup(3), [0, 0, 0, 0]);

        // Drop PLTE → None.
        let mut nopal = Vec::new();
        nopal.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        nopal.extend_from_slice(&13u32.to_be_bytes());
        nopal.extend_from_slice(b"IHDR");
        nopal.extend_from_slice(&[0u8; 13]);
        nopal.extend_from_slice(&[0u8; 4]);
        assert!(
            read_png_palette(&nopal).is_none(),
            "palette-less PNG must return None"
        );

        // Wrong signature → None.
        let mut bad = buf.clone();
        bad[0] = 0;
        assert!(read_png_palette(&bad).is_none());
    }

    /// `videoframe_to_rgba` with a palette translates the 1-byte plane
    /// indices into the right RGBA values.
    #[test]
    fn videoframe_pal8_to_bitmap_via_palette() {
        let frame = VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: 2, // 2 px * 1 B
                data: vec![
                    0, 1, // (0,0) idx 0, (1,0) idx 1
                    2, 1, // (0,1) idx 2, (1,1) idx 1
                ],
            }],
        };
        let pal = PngPalette {
            entries: vec![
                [255, 0, 0, 255], // 0 → red opaque
                [0, 255, 0, 128], // 1 → green half-alpha
                [0, 0, 255, 255], // 2 → blue opaque
            ],
        };
        let bm = videoframe_to_rgba(&frame, 2, 2, Some(&pal));
        assert_eq!(bm.get(0, 0), [255, 0, 0, 255]);
        assert_eq!(bm.get(1, 0), [0, 255, 0, 128]);
        assert_eq!(bm.get(0, 1), [0, 0, 255, 255]);
        assert_eq!(bm.get(1, 1), [0, 255, 0, 128]);
    }

    /// `videoframe_to_rgba` with no palette continues to splat the
    /// 1-byte plane as grayscale (Gray8 path) — back-compat for
    /// non-palette grayscale PNGs.
    #[test]
    fn videoframe_gray8_to_bitmap_without_palette() {
        let frame = VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: 2,
                data: vec![100, 200, 50, 25],
            }],
        };
        let bm = videoframe_to_rgba(&frame, 2, 2, None);
        assert_eq!(bm.get(0, 0), [100, 100, 100, 255]);
        assert_eq!(bm.get(1, 0), [200, 200, 200, 255]);
        assert_eq!(bm.get(0, 1), [50, 50, 50, 255]);
        assert_eq!(bm.get(1, 1), [25, 25, 25, 255]);
    }
}
