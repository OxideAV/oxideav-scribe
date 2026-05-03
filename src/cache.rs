//! Tiny LRU cache for rasterised glyph alpha bitmaps.
//!
//! Subtitle renderers typically reuse the same ~50 unique glyphs per
//! cue line; even a 256-entry LRU achieves a >95% hit rate. The
//! implementation is a `Vec<(Key, Value)>` ordered by recency: the
//! most-recently-used entry sits at index 0. Insertion costs O(n) for
//! the rotation but n is bounded at 256 and amortises away under the
//! hit rate.

use std::collections::VecDeque;

use crate::rasterizer::AlphaBitmap;

/// Cache key. `size_q8` is `(size_px * 256.0).round() as u32` so that
/// two requests one quarter-pixel apart still hit the same entry,
/// while distinct integer sizes (the common case) live in separate
/// slots. `shear_q14` is the requested italic shear, similarly
/// quantised, so that synthesised-italic glyphs never collide with
/// upright ones in the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub face_id: u64,
    pub glyph_id: u16,
    pub size_q8: u32,
    /// Quantised shear (`tan(angle) * 16384` as i32). 0 for upright.
    /// 14 fractional bits give >0.0001 resolution, well below visible
    /// quantisation across any sane synthetic-italic angle.
    pub shear_q14: i32,
}

impl GlyphKey {
    /// Build a key from a face id, glyph id, and pixel size. No shear.
    pub fn new(face_id: u64, glyph_id: u16, size_px: f32) -> Self {
        Self::new_styled(face_id, glyph_id, size_px, 0.0)
    }

    /// Build a key with a horizontal shear (synthetic italic).
    pub fn new_styled(face_id: u64, glyph_id: u16, size_px: f32, shear_x_per_y: f32) -> Self {
        let size_q8 = (size_px * 256.0).round().max(0.0) as u32;
        let shear_q14 = (shear_x_per_y * 16384.0).round() as i32;
        Self {
            face_id,
            glyph_id,
            size_q8,
            shear_q14,
        }
    }
}

/// Cached glyph entry: the bitmap plus the bbox-relative offset to its
/// pen origin so the composer can place it correctly without re-running
/// the full flatten step.
#[derive(Debug, Clone)]
pub struct CachedGlyph {
    pub bitmap: AlphaBitmap,
    /// Pen-origin → bitmap top-left offset, in raster pixels (Y-down).
    pub offset_x: f32,
    pub offset_y: f32,
}

/// Default LRU capacity. Subtitle traffic comfortably fits in 256.
pub const DEFAULT_CAPACITY: usize = 256;

/// Move-to-front LRU. `entries[0]` is the freshest entry.
#[derive(Debug)]
pub struct GlyphCache {
    entries: VecDeque<(GlyphKey, CachedGlyph)>,
    capacity: usize,
    hits: u64,
    misses: u64,
}

impl GlyphCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity.max(1)),
            capacity: capacity.max(1),
            hits: 0,
            misses: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn hits(&self) -> u64 {
        self.hits
    }
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// Look up a key. If present, marks the entry as freshest and
    /// returns a clone (cheap — `AlphaBitmap` is just a `Vec<u8>` and
    /// the caller usually only blits it once).
    pub fn get(&mut self, key: &GlyphKey) -> Option<CachedGlyph> {
        let pos = self.entries.iter().position(|(k, _)| k == key);
        match pos {
            Some(0) => {
                self.hits += 1;
                Some(self.entries[0].1.clone())
            }
            Some(i) => {
                self.hits += 1;
                let entry = self.entries.remove(i).expect("position in range");
                let val = entry.1.clone();
                self.entries.push_front(entry);
                Some(val)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Insert `value` for `key`. If the key already exists it is moved
    /// to the front and updated; otherwise the eldest entry is evicted
    /// when capacity would be exceeded.
    pub fn insert(&mut self, key: GlyphKey, value: CachedGlyph) {
        if let Some(i) = self.entries.iter().position(|(k, _)| k == &key) {
            self.entries.remove(i);
        }
        self.entries.push_front((key, value));
        while self.entries.len() > self.capacity {
            self.entries.pop_back();
        }
    }
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy(w: u32, h: u32) -> CachedGlyph {
        CachedGlyph {
            bitmap: AlphaBitmap::new(w, h),
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }

    #[test]
    fn key_quantises_to_q8() {
        let k1 = GlyphKey::new(7, 42, 16.0);
        let k2 = GlyphKey::new(7, 42, 16.001);
        assert_eq!(k1, k2);
        let k3 = GlyphKey::new(7, 42, 16.5);
        assert_ne!(k1, k3);
    }

    #[test]
    fn lru_evicts_eldest() {
        let mut c = GlyphCache::new(2);
        c.insert(GlyphKey::new(0, 1, 16.0), dummy(2, 2));
        c.insert(GlyphKey::new(0, 2, 16.0), dummy(2, 2));
        c.insert(GlyphKey::new(0, 3, 16.0), dummy(2, 2));
        assert!(c.get(&GlyphKey::new(0, 1, 16.0)).is_none());
        assert!(c.get(&GlyphKey::new(0, 2, 16.0)).is_some());
        assert!(c.get(&GlyphKey::new(0, 3, 16.0)).is_some());
    }

    #[test]
    fn get_promotes_to_front() {
        let mut c = GlyphCache::new(2);
        c.insert(GlyphKey::new(0, 1, 16.0), dummy(2, 2));
        c.insert(GlyphKey::new(0, 2, 16.0), dummy(2, 2));
        // Touch 1 — now 1 is freshest, 2 is eldest.
        let _ = c.get(&GlyphKey::new(0, 1, 16.0));
        c.insert(GlyphKey::new(0, 3, 16.0), dummy(2, 2));
        assert!(c.get(&GlyphKey::new(0, 1, 16.0)).is_some());
        assert!(c.get(&GlyphKey::new(0, 2, 16.0)).is_none());
    }
}
