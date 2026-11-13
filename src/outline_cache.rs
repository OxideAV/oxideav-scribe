//! Optional bounded cache of resolved glyph outlines.
//!
//! Decoding a glyph outline walks the `glyf` / `CFF ` data, resolves
//! composite components, applies `gvar` deltas for a variation
//! instance and converts the result to an [`oxideav_core::Path`]. The
//! result depends only on *(face, glyph id, variation instance)* — not
//! on the size or position the glyph is drawn at — so a UI that draws
//! the same few hundred glyphs every frame, at several sizes, can
//! decode each once and reuse the font-unit path.
//!
//! [`OutlineCache`] is that memo. It is **off by default**: a
//! [`crate::Face`] only consults a cache the caller attached with
//! [`crate::Face::set_outline_cache`] (or
//! [`crate::FaceChain::set_outline_cache`]). One cache may be shared
//! by any number of faces and chains.
//!
//! ## Keys — why a hit is always the right glyph
//!
//! An entry is keyed by
//!
//! * the face's [`crate::Face::id`] — a process-wide counter value
//!   that is never reused, so two faces can never alias, not even two
//!   faces built from the same bytes, and not after one is dropped;
//! * the glyph id;
//! * the face's variation instance — the exact bit patterns of the
//!   installed user-space coordinates (absent for the default
//!   instance). The whole vector is compared on every lookup; there is
//!   no lossy digest that two instances could share.
//!
//! Because the instance is part of the key, changing a face's
//! variation coordinates needs **no invalidation**: the new instance
//! simply misses, and the old instance's entries are found again if
//! the face returns to it (or age out if it does not).
//!
//! ## Memory bound
//!
//! The cache is bounded by a byte budget
//! ([`OutlineCache::new`]; [`DEFAULT_BUDGET_BYTES`] for
//! [`OutlineCache::default`]). Each entry is charged for its path
//! commands plus a fixed bookkeeping allowance. Entries live in two
//! generations, each capped at half the budget: new entries (and
//! entries hit in the old generation) go to the young one; when it is
//! full the old generation is dropped wholesale and the young one
//! takes its place. Recently used outlines therefore survive, the
//! charged total never exceeds the budget, and eviction costs no
//! per-entry bookkeeping. A single outline larger than half the budget
//! is returned to the caller but not retained.
//!
//! The paths are handed out as `Arc<Path>`; a caller that keeps one
//! keeps that allocation alive after the cache has evicted it — such
//! memory is the caller's, not counted against the budget.
//!
//! ## Threads
//!
//! `OutlineCache` is `Send + Sync`. Lookups and inserts take a short
//! internal lock; the outline decode itself runs outside it, so two
//! threads that miss on the same glyph at the same time both decode it
//! (and store identical results) rather than one blocking the other.

use oxideav_core::{Path, PathCommand};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// Byte budget of [`OutlineCache::default`]: 4 MiB — room for a few
/// thousand typical Latin outlines or several hundred dense CJK ones.
pub const DEFAULT_BUDGET_BYTES: usize = 4 << 20;

/// Fixed per-entry charge on top of the path commands: the key, the
/// slot, the `Arc` / `Path` headers and the map's own slack.
const ENTRY_OVERHEAD_BYTES: usize = 96;

/// The variation instance a cached outline was resolved at: the bit
/// patterns of the face's installed user-space coordinates, or nothing
/// for the default instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub(crate) struct InstanceKey(Option<Arc<[u32]>>);

impl InstanceKey {
    /// Key for an installed coordinate vector (empty → default).
    pub(crate) fn from_coords(coords: &[f32]) -> Self {
        if coords.is_empty() {
            Self(None)
        } else {
            Self(Some(coords.iter().map(|c| c.to_bits()).collect()))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    face: u64,
    glyph: u16,
    instance: InstanceKey,
}

#[derive(Debug, Clone)]
struct Slot {
    /// `None` records "this glyph has no outline" (whitespace, bitmap
    /// only, out of range) so that is not re-derived either.
    path: Option<Arc<Path>>,
    cost: usize,
}

#[derive(Debug, Default)]
struct Generations {
    young: HashMap<Key, Slot>,
    old: HashMap<Key, Slot>,
    young_bytes: usize,
    old_bytes: usize,
}

impl Generations {
    /// Put `slot` in the young generation, retiring the old one first
    /// if the young one would overflow its half of the budget.
    fn admit(&mut self, key: Key, slot: Slot, half_budget: usize) {
        if slot.cost > half_budget {
            return;
        }
        if self.young_bytes + slot.cost > half_budget {
            self.old = std::mem::take(&mut self.young);
            self.old_bytes = self.young_bytes;
            self.young_bytes = 0;
        }
        let cost = slot.cost;
        if let Some(prev) = self.young.insert(key, slot) {
            self.young_bytes -= prev.cost;
        }
        self.young_bytes += cost;
    }
}

/// Counters and occupancy of an [`OutlineCache`], from
/// [`OutlineCache::stats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct OutlineCacheStats {
    /// Lookups answered from the cache.
    pub hits: u64,
    /// Lookups that had to decode the outline.
    pub misses: u64,
    /// Entries currently held (both generations).
    pub entries: usize,
    /// Bytes currently charged against the budget.
    pub bytes: usize,
}

/// Bounded, thread-safe memo of glyph outlines in font units, keyed by
/// *(face, glyph id, variation instance)*. See the [module
/// docs](self) for the key, the memory bound and the threading rules.
///
/// ```no_run
/// # use oxideav_scribe::{Face, FaceChain, OutlineCache};
/// # use std::sync::Arc;
/// # fn demo(face: Face) {
/// let cache = Arc::new(OutlineCache::new(2 << 20)); // 2 MiB
/// let mut chain = FaceChain::new(face);
/// chain.set_outline_cache(Some(cache.clone()));
/// // Every `glyph_path` / `glyph_node` / `shape_to_paths` on the
/// // chain's faces now decodes each glyph once.
/// # }
/// ```
#[derive(Debug)]
pub struct OutlineCache {
    generations: Mutex<Generations>,
    half_budget: usize,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl Default for OutlineCache {
    fn default() -> Self {
        Self::new(DEFAULT_BUDGET_BYTES)
    }
}

impl OutlineCache {
    /// A cache that retains at most `budget_bytes` of outlines. A
    /// budget of `0` retains nothing (every lookup decodes).
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            generations: Mutex::new(Generations::default()),
            half_budget: budget_bytes / 2,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// The byte budget this cache was built with (rounded down to an
    /// even number).
    pub fn budget_bytes(&self) -> usize {
        self.half_budget * 2
    }

    /// Hit / miss counters and current occupancy.
    pub fn stats(&self) -> OutlineCacheStats {
        let g = self.lock();
        OutlineCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            entries: g.young.len() + g.old.len(),
            bytes: g.young_bytes + g.old_bytes,
        }
    }

    /// Drop every entry. The hit / miss counters keep counting.
    pub fn clear(&self) {
        *self.lock() = Generations::default();
    }

    /// Drop every entry of the face whose [`crate::Face::id`] is
    /// `face_id`. Never needed for correctness (ids are not reused);
    /// it only releases the memory sooner than ageing would.
    pub fn forget_face(&self, face_id: u64) {
        let mut g = self.lock();
        let g = &mut *g;
        for (map, bytes) in [
            (&mut g.young, &mut g.young_bytes),
            (&mut g.old, &mut g.old_bytes),
        ] {
            map.retain(|k, slot| {
                let keep = k.face != face_id;
                if !keep {
                    *bytes -= slot.cost;
                }
                keep
            });
        }
    }

    fn lock(&self) -> MutexGuard<'_, Generations> {
        // The guarded maps are only ever updated by whole-entry
        // operations, so a panic on another thread cannot leave them
        // half-written; a poisoned lock is safe to keep using.
        self.generations.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The outline of `(face, glyph, instance)`, decoding it with
    /// `decode` on a miss. `decode` runs without the lock held.
    pub(crate) fn get_or_decode(
        &self,
        face: u64,
        glyph: u16,
        instance: &InstanceKey,
        decode: impl FnOnce() -> Option<Path>,
    ) -> Option<Arc<Path>> {
        let key = Key {
            face,
            glyph,
            instance: instance.clone(),
        };
        {
            let mut g = self.lock();
            if let Some(slot) = g.young.get(&key) {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return slot.path.clone();
            }
            if let Some(slot) = g.old.remove(&key) {
                // Still in use: carry it over to the young generation.
                g.old_bytes -= slot.cost;
                let path = slot.path.clone();
                g.admit(key, slot, self.half_budget);
                self.hits.fetch_add(1, Ordering::Relaxed);
                return path;
            }
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        let path = decode().map(Arc::new);
        let cost = ENTRY_OVERHEAD_BYTES
            + path
                .as_ref()
                .map_or(0, |p| p.commands.len() * std::mem::size_of::<PathCommand>());
        let slot = Slot {
            path: path.clone(),
            cost,
        };
        self.lock().admit(key, slot, self.half_budget);
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::Point;

    fn path(n: usize, tag: f32) -> Path {
        Path {
            commands: (0..n)
                .map(|i| PathCommand::LineTo(Point::new(tag, i as f32)))
                .collect(),
        }
    }

    fn inst(c: &[f32]) -> InstanceKey {
        InstanceKey::from_coords(c)
    }

    fn cost(n: usize) -> usize {
        ENTRY_OVERHEAD_BYTES + n * std::mem::size_of::<PathCommand>()
    }

    #[test]
    fn cache_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<OutlineCache>();
    }

    #[test]
    fn hit_returns_the_stored_path_without_decoding() {
        let c = OutlineCache::default();
        let a = c.get_or_decode(1, 7, &inst(&[]), || Some(path(4, 1.0)));
        let b = c.get_or_decode(1, 7, &inst(&[]), || panic!("must not decode on a hit"));
        assert!(Arc::ptr_eq(a.as_ref().unwrap(), b.as_ref().unwrap()));
        let s = c.stats();
        assert_eq!((s.hits, s.misses, s.entries), (1, 1, 1));
        assert_eq!(s.bytes, cost(4));
    }

    #[test]
    fn absent_outlines_are_remembered_too() {
        let c = OutlineCache::default();
        assert!(c.get_or_decode(1, 3, &inst(&[]), || None).is_none());
        assert!(
            c.get_or_decode(1, 3, &inst(&[]), || panic!("negative entry must hit"))
                .is_none()
        );
        assert_eq!(c.stats().bytes, ENTRY_OVERHEAD_BYTES);
    }

    #[test]
    fn face_glyph_and_instance_all_separate_entries() {
        let c = OutlineCache::default();
        let variants: Vec<(u64, u16, InstanceKey)> = vec![
            (1, 7, inst(&[])),
            (2, 7, inst(&[])),
            (1, 8, inst(&[])),
            (1, 7, inst(&[400.0])),
            (1, 7, inst(&[700.0])),
            (1, 7, inst(&[400.0, 14.0])),
            // Same value, different bit pattern: still its own entry.
            (1, 7, inst(&[0.0])),
            (1, 7, inst(&[-0.0])),
        ];
        for (i, (f, g, k)) in variants.iter().enumerate() {
            let p = c.get_or_decode(*f, *g, k, || Some(path(2, i as f32)));
            assert_eq!(p.unwrap().commands, path(2, i as f32).commands);
        }
        assert_eq!(c.stats().misses, variants.len() as u64);
        for (i, (f, g, k)) in variants.iter().enumerate() {
            let p = c.get_or_decode(*f, *g, k, || panic!("hit expected"));
            assert_eq!(p.unwrap().commands, path(2, i as f32).commands);
        }
    }

    #[test]
    fn charged_bytes_never_exceed_the_budget() {
        let budget = 20 * cost(8);
        let c = OutlineCache::new(budget);
        for gid in 0..1000u16 {
            c.get_or_decode(1, gid, &inst(&[]), || Some(path(8, gid as f32)));
            let s = c.stats();
            assert!(s.bytes <= budget, "{} > {budget}", s.bytes);
            assert!(s.entries <= 20);
        }
        // The most recent entries are still there.
        c.get_or_decode(1, 999, &inst(&[]), || panic!("recent entry evicted"));
    }

    #[test]
    fn entries_in_use_survive_a_generation_turnover() {
        let c = OutlineCache::new(8 * cost(1));
        // Each generation holds four one-command entries.
        c.get_or_decode(1, 0, &inst(&[]), || Some(path(1, 0.0)));
        for gid in 1..40u16 {
            c.get_or_decode(1, gid, &inst(&[]), || Some(path(1, gid as f32)));
            // Glyph 0 is touched between every insert, so it is
            // carried forward each time its generation retires.
            let p = c.get_or_decode(1, 0, &inst(&[]), || panic!("hot entry evicted"));
            assert_eq!(p.unwrap().commands, path(1, 0.0).commands);
        }
        // A glyph that was not touched again has aged out.
        let mut decoded = false;
        c.get_or_decode(1, 1, &inst(&[]), || {
            decoded = true;
            Some(path(1, 1.0))
        });
        assert!(decoded);
    }

    #[test]
    fn oversized_and_zero_budget_entries_are_not_retained() {
        let c = OutlineCache::new(2 * cost(4));
        let p = c.get_or_decode(1, 1, &inst(&[]), || Some(path(500, 1.0)));
        assert_eq!(p.unwrap().commands.len(), 500);
        assert_eq!(c.stats().entries, 0);

        let none = OutlineCache::new(0);
        none.get_or_decode(1, 1, &inst(&[]), || Some(path(1, 1.0)));
        none.get_or_decode(1, 1, &inst(&[]), || Some(path(1, 1.0)));
        let s = none.stats();
        assert_eq!((s.hits, s.misses, s.entries, s.bytes), (0, 2, 0, 0));
    }

    #[test]
    fn forget_face_and_clear_release_their_bytes() {
        let c = OutlineCache::new(1 << 16);
        for gid in 0..10u16 {
            c.get_or_decode(1, gid, &inst(&[]), || Some(path(3, 1.0)));
            c.get_or_decode(2, gid, &inst(&[]), || Some(path(3, 2.0)));
        }
        assert_eq!(c.stats().bytes, 20 * cost(3));
        c.forget_face(1);
        let s = c.stats();
        assert_eq!((s.entries, s.bytes), (10, 10 * cost(3)));
        c.get_or_decode(2, 4, &inst(&[]), || panic!("other face must stay"));
        c.clear();
        let s = c.stats();
        assert_eq!((s.entries, s.bytes), (0, 0));
        assert_eq!(c.budget_bytes(), 1 << 16);
    }

    #[test]
    fn concurrent_lookups_agree() {
        let c = Arc::new(OutlineCache::new(64 * cost(4)));
        std::thread::scope(|s| {
            for t in 0..4u64 {
                let c = &c;
                s.spawn(move || {
                    for round in 0..200u16 {
                        for face in 1..=3u64 {
                            let gid = (round * 7 + t as u16) % 50;
                            let want = path(4, face as f32 * 1000.0 + gid as f32);
                            let got = c
                                .get_or_decode(face, gid, &inst(&[]), || Some(want.clone()))
                                .unwrap();
                            assert_eq!(got.commands, want.commands);
                        }
                    }
                });
            }
        });
        let s = c.stats();
        assert!(s.bytes <= c.budget_bytes());
        assert_eq!(s.hits + s.misses, 4 * 200 * 3);
    }
}
