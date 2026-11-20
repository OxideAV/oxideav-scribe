//! Per-face layout plan: the lookup-table facts shaping needs on every
//! call, derived from the font once.
//!
//! Shaping a run asks the font the same structural questions each
//! time — which lookup has which type, which features (and so which
//! lookups) a script publishes, which `GPOS` lookups are single /
//! cursive / contextual positioning. The parser answers each by
//! walking the `GSUB` / `GPOS` ScriptList, FeatureList and LookupList
//! and building a fresh list. None of the answers depends on the text
//! or on the variation instance, so a [`LayoutPlan`] computes each one
//! on first use and keeps it; a [`crate::Face`] holds one plan for its
//! whole life.
//!
//! A plan is only ever valid for the font it was filled from. The
//! font-taking public entry points build a throw-away plan per call;
//! the `Face` / `FaceChain` entry points pass the face's own.

use crate::shaping::staged::StageLookup;
use oxideav_ttf::Font;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// One feature record of a script's default language system.
#[derive(Debug, Clone)]
pub(crate) struct PlannedFeature {
    pub(crate) tag: [u8; 4],
    pub(crate) lookups: Vec<u16>,
}

/// The `GPOS` lookups the positioning pass dispatches by type.
#[derive(Debug, Default)]
pub(crate) struct GposPlan {
    /// SinglePos (type 1) lookups, in lookup-list order.
    pub(crate) single: Vec<u16>,
    /// Cursive-attachment (type 3) lookups as `(lookup index, whether
    /// the lookup's RIGHT_TO_LEFT flag is set)`, in lookup-list order.
    pub(crate) cursive: Vec<(u16, bool)>,
    /// Contextual / chained-contextual (type 7 / 8) lookups as
    /// `(lookup index, type)`, in lookup-list order.
    pub(crate) contextual: Vec<(u16, u16)>,
}

/// `lookupFlag` bit 0x0001 — used only by cursive attachment (GPOS
/// lookup type 3): the last glyph of a connected sequence stays on the
/// baseline and the preceding ones are adjusted.
const RIGHT_TO_LEFT: u16 = 0x0001;

/// Most scripts whose feature lists one plan keeps (the OpenType
/// script-tag registry has under two hundred entries; a font uses a
/// handful).
const MAX_KEPT_SCRIPTS: usize = 256;

/// Script tags the always-on `ccmp` / `calt` passes probe, in priority
/// order; the first one publishing the feature wins.
const GENERAL_SCRIPT_TAGS: [[u8; 4]; 4] = [*b"latn", *b"cyrl", *b"grek", *b"DFLT"];

/// Lazily filled, thread-safe layout facts of one font.
#[derive(Debug, Default)]
pub(crate) struct LayoutPlan {
    /// `(lookup index, effective type)` for every `GSUB` lookup.
    gsub_types: OnceLock<Vec<(u16, u16)>>,
    gpos_types: OnceLock<Vec<(u16, u16)>>,
    gpos: OnceLock<GposPlan>,
    ccmp: OnceLock<Vec<u16>>,
    calt: OnceLock<Vec<u16>>,
    /// `GSUB` features of each script's default language system.
    script_features: Mutex<HashMap<[u8; 4], Arc<[PlannedFeature]>>>,
    /// GPOS mirror of `script_features`.
    gpos_script_features: Mutex<HashMap<[u8; 4], Arc<[PlannedFeature]>>>,
    /// Per script tag: which positioning lookup types its features
    /// reach (bit `n` set = a lookup of type `n`).
    gpos_script_types: Mutex<HashMap<[u8; 4], u16>>,
    /// GDEF mark class per glyph, learned as glyphs are asked about.
    marks: Mutex<GlyphSet>,
    /// Per `(lookupFlag, markFilteringSet)`: the glyphs a lookup with
    /// those settings skips, learned as glyphs are asked about.
    skip_sets: Mutex<HashMap<(u16, Option<u16>), GlyphSet>>,
    /// Results of single-substitution lookups (see
    /// [`LayoutPlan::single_substitute`]).
    single_memo: OnceLock<Box<[AtomicU64]>>,
    /// Resolved stages of the staged engine, keyed by a digest of the
    /// request; the request is kept to rule out a digest collision.
    stages: Mutex<HashMap<u64, KeptStage>>,
    /// Which consonant forms the font has (the Indic procedure's
    /// "dynamic character properties"), keyed by script tag, feature
    /// and the two characters tried.
    forms: Mutex<HashMap<FormKey, bool>>,
}

/// `(script tag, feature tag, first character, second character)`.
pub(crate) type FormKey = ([u8; 4], [u8; 4], char, char);

/// One resolved stage and the request it answers.
type KeptStage = (StageRequest, Arc<[StageLookup]>);

/// `(positioning table?, script tag, features of the stage)`.
type StageRequest = (bool, [u8; 4], Vec<([u8; 4], u32, u32)>);

/// A set of glyph ids with a "known" bit per glyph, for per-glyph
/// facts that are computed on first demand and never change.
#[derive(Debug, Default)]
pub(crate) struct GlyphSet {
    known: Vec<u64>,
    member: Vec<u64>,
}

impl GlyphSet {
    /// The kept answer for `gid`, or the result of `compute`, which
    /// is then kept.
    #[inline]
    pub(crate) fn get_or(&mut self, gid: u16, compute: impl FnOnce() -> bool) -> bool {
        let (word, bit) = (usize::from(gid) / 64, 1u64 << (gid % 64));
        if word >= self.known.len() {
            self.known.resize(word + 1, 0);
            self.member.resize(word + 1, 0);
        }
        if self.known[word] & bit == 0 {
            self.known[word] |= bit;
            if compute() {
                self.member[word] |= bit;
            }
        }
        self.member[word] & bit != 0
    }
}

/// Slots of the single-substitution memo (a power of two).
const SINGLE_MEMO_SLOTS: usize = 4096;

impl LayoutPlan {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The effective type of `GSUB` lookup `index`.
    pub(crate) fn gsub_lookup_type(&self, font: &Font<'_>, index: u16) -> Option<u16> {
        let types = self.gsub_types.get_or_init(|| {
            font.gsub_lookup_list()
                .into_iter()
                .map(|(i, ty, _)| (i, ty))
                .collect()
        });
        // The list is in lookup-index order, so the entry for `index`
        // normally sits at that position.
        match types.get(index as usize) {
            Some(&(i, ty)) if i == index => Some(ty),
            _ => types.iter().find(|&&(i, _)| i == index).map(|&(_, ty)| ty),
        }
    }

    /// The `GPOS` lookups grouped the way the positioning pass uses
    /// them.
    /// Effective type of GPOS lookup `index` (extension lookups
    /// unwrapped).
    pub(crate) fn gpos_lookup_type(&self, font: &Font<'_>, index: u16) -> Option<u16> {
        let types = self.gpos_types.get_or_init(|| {
            font.gpos_lookup_list()
                .into_iter()
                .map(|(i, ty, _)| (i, ty))
                .collect()
        });
        match types.get(index as usize) {
            Some(&(i, ty)) if i == index => Some(ty),
            _ => types.iter().find(|&&(i, _)| i == index).map(|&(_, ty)| ty),
        }
    }

    pub(crate) fn gpos(&self, font: &Font<'_>) -> &GposPlan {
        self.gpos.get_or_init(|| {
            let mut plan = GposPlan::default();
            for (index, ty, _) in font.gpos_lookup_list() {
                match ty {
                    1 => plan.single.push(index),
                    3 => plan
                        .cursive
                        .push((index, font.gpos_lookup_flags(index) & RIGHT_TO_LEFT != 0)),
                    7 | 8 => plan.contextual.push((index, ty)),
                    _ => {}
                }
            }
            plan
        })
    }

    /// The `GSUB` features `script_tag` publishes under its default
    /// language system, in declaration order.
    pub(crate) fn gsub_features(
        &self,
        font: &Font<'_>,
        script_tag: [u8; 4],
    ) -> Arc<[PlannedFeature]> {
        // Whole-entry updates only, so a poisoned lock is still
        // consistent.
        if let Some(hit) = self
            .script_features
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&script_tag)
        {
            return hit.clone();
        }
        let features: Arc<[PlannedFeature]> = font
            .gsub_features_for_script(script_tag, None)
            .into_iter()
            .map(|f| PlannedFeature {
                tag: f.tag,
                lookups: f.lookup_indices,
            })
            .collect();
        let mut kept = self
            .script_features
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Callers choose the tags they ask about; keep a bounded number
        // of answers so arbitrary tags cannot grow the plan without
        // limit. Past the bound a query is simply resolved per call.
        if kept.len() < MAX_KEPT_SCRIPTS {
            kept.insert(script_tag, features.clone());
        }
        features
    }

    /// Lookup indices of every `feature_tag` record `script_tag`
    /// publishes, in declaration order.
    /// The GPOS features (default language system) of `script_tag`,
    /// resolved once per tag.
    pub(crate) fn gpos_features(
        &self,
        font: &Font<'_>,
        script_tag: [u8; 4],
    ) -> Arc<[PlannedFeature]> {
        if let Some(hit) = self
            .gpos_script_features
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&script_tag)
        {
            return hit.clone();
        }
        let features: Arc<[PlannedFeature]> = font
            .gpos_features_for_script(script_tag, None)
            .into_iter()
            .map(|f| PlannedFeature {
                tag: f.tag,
                lookups: f.lookup_indices,
            })
            .collect();
        let mut kept = self
            .gpos_script_features
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if kept.len() < MAX_KEPT_SCRIPTS {
            kept.insert(script_tag, features.clone());
        }
        features
    }

    /// Bit set of the GPOS lookup types reachable from the features
    /// of `script_tag` (bit `n` = lookup type `n`).
    pub(crate) fn gpos_types_for_script(&self, font: &Font<'_>, script_tag: [u8; 4]) -> u16 {
        if let Some(&hit) = self
            .gpos_script_types
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&script_tag)
        {
            return hit;
        }
        let mut types = 0u16;
        for feature in self.gpos_features(font, script_tag).iter() {
            for &index in &feature.lookups {
                if let Some(ty) = self.gpos_lookup_type(font, index)
                    && ty < 16
                {
                    types |= 1 << ty;
                }
            }
        }
        let mut kept = self
            .gpos_script_types
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if kept.len() < MAX_KEPT_SCRIPTS {
            kept.insert(script_tag, types);
        }
        types
    }

    /// GDEF mark class (class 3) of every glyph of `gids`, written to
    /// `out` (cleared first). One lock for the whole batch; each glyph
    /// is classified by the font once per face.
    pub(crate) fn marks_of(
        &self,
        font: &Font<'_>,
        gids: impl Iterator<Item = u16>,
        out: &mut Vec<bool>,
    ) {
        out.clear();
        let mut marks = self.marks.lock().unwrap_or_else(|e| e.into_inner());
        out.extend(gids.map(|g| marks.get_or(g, || font.is_mark_glyph(g))));
    }

    /// Run `visit` with the kept skip set of a lookup's
    /// `(lookupFlag, markFilteringSet)` pair: `visit` gets a function
    /// answering "does such a lookup skip this glyph?", each glyph
    /// being asked of the font once per face.
    pub(crate) fn with_skips<R>(
        &self,
        font: &Font<'_>,
        flags: u16,
        mark_filtering_set: Option<u16>,
        visit: impl FnOnce(&mut dyn FnMut(u16) -> bool) -> R,
    ) -> R {
        let mut sets = self.skip_sets.lock().unwrap_or_else(|e| e.into_inner());
        if sets.len() >= MAX_KEPT_SCRIPTS && !sets.contains_key(&(flags, mark_filtering_set)) {
            // A hostile font with thousands of flag combinations:
            // answer without keeping.
            drop(sets);
            return visit(&mut |g| font.lookup_skips_glyph(flags, mark_filtering_set, g));
        }
        let set = sets.entry((flags, mark_filtering_set)).or_default();
        visit(&mut |g| set.get_or(g, || font.lookup_skips_glyph(flags, mark_filtering_set, g)))
    }

    /// `Font::gsub_apply_lookup_type_1` through a small per-face memo:
    /// a single substitution depends on the lookup and the glyph
    /// only, and shaping asks the same few hundred pairs over and
    /// over (every glyph of a run against every active lookup).
    pub(crate) fn single_substitute(&self, font: &Font<'_>, lookup: u16, gid: u16) -> Option<u16> {
        const VALID: u64 = 1 << 63;
        const HIT: u64 = 1 << 62;
        let memo = self.single_memo.get_or_init(|| {
            (0..SINGLE_MEMO_SLOTS)
                .map(|_| AtomicU64::new(0))
                .collect::<Vec<_>>()
                .into_boxed_slice()
        });
        let key = (u32::from(lookup) << 16) | u32::from(gid);
        let slot = &memo[(key.wrapping_mul(0x9E37_79B1) >> 20) as usize & (SINGLE_MEMO_SLOTS - 1)];
        let entry = slot.load(Ordering::Relaxed);
        if entry & VALID != 0 && ((entry >> 16) & 0xFFFF_FFFF) as u32 == key {
            return (entry & HIT != 0).then_some(entry as u16);
        }
        let result = font.gsub_apply_lookup_type_1(lookup, gid);
        let packed = VALID
            | if result.is_some() { HIT } else { 0 }
            | (u64::from(key) << 16)
            | u64::from(result.unwrap_or(0));
        slot.store(packed, Ordering::Relaxed);
        result
    }

    /// The kept resolution of one stage of the staged engine for a
    /// script tag (default language system), built on first request.
    pub(crate) fn stage(
        &self,
        positioning: bool,
        script_tag: [u8; 4],
        features: &[([u8; 4], u32, u32)],
        build: impl FnOnce() -> Vec<StageLookup>,
    ) -> Arc<[StageLookup]> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (positioning, script_tag, features).hash(&mut h);
        let digest = h.finish();
        if let Some((request, kept)) = self
            .stages
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&digest)
            && request.0 == positioning
            && request.1 == script_tag
            && request.2 == features
        {
            return kept.clone();
        }
        let built: Arc<[StageLookup]> = build().into();
        let mut stages = self.stages.lock().unwrap_or_else(|e| e.into_inner());
        if stages.len() < MAX_KEPT_SCRIPTS {
            stages.insert(
                digest,
                ((positioning, script_tag, features.to_vec()), built.clone()),
            );
        }
        built
    }

    /// The kept answer to "does the font have this consonant form?",
    /// or the result of `probe`, which is then kept.
    pub(crate) fn form(&self, key: FormKey, probe: impl FnOnce() -> bool) -> bool {
        if let Some(&hit) = self
            .forms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
        {
            return hit;
        }
        let formed = probe();
        let mut forms = self.forms.lock().unwrap_or_else(|e| e.into_inner());
        if forms.len() < 4096 {
            forms.insert(key, formed);
        }
        formed
    }

    pub(crate) fn gsub_feature_lookups(
        &self,
        font: &Font<'_>,
        script_tag: [u8; 4],
        feature_tag: &[u8; 4],
    ) -> Vec<u16> {
        let mut hits = Vec::new();
        for feature in self.gsub_features(font, script_tag).iter() {
            if &feature.tag == feature_tag {
                hits.extend_from_slice(&feature.lookups);
            }
        }
        hits
    }

    /// Lookups of an always-on general-script feature: those of the
    /// first of `latn` / `cyrl` / `grek` / `DFLT` that publishes
    /// `feature_tag`. `ccmp` and `calt` are kept; other tags are
    /// resolved per call.
    pub(crate) fn general_lookups<'p>(
        &'p self,
        font: &Font<'_>,
        feature_tag: &[u8; 4],
    ) -> std::borrow::Cow<'p, [u16]> {
        let resolve = || {
            GENERAL_SCRIPT_TAGS
                .iter()
                .map(|&tag| self.gsub_feature_lookups(font, tag, feature_tag))
                .find(|hits| !hits.is_empty())
                .unwrap_or_default()
        };
        match feature_tag {
            b"ccmp" => self.ccmp.get_or_init(resolve).as_slice().into(),
            b"calt" => self.calt.get_or_init(resolve).as_slice().into(),
            _ => resolve().into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asking about arbitrarily many script tags does not grow the
    /// plan past its bound, and answers stay correct beyond it.
    #[test]
    fn memos_answer_like_the_font() {
        for face in fonts() {
            let font = face.ttf().unwrap();
            let plan = LayoutPlan::new();
            let glyphs = font.glyph_count().min(3000);
            // Mark classes, asked twice (learned, then kept).
            for _ in 0..2 {
                let mut marks = Vec::new();
                plan.marks_of(font, 0..glyphs, &mut marks);
                for g in 0..glyphs {
                    assert_eq!(marks[usize::from(g)], font.is_mark_glyph(g), "glyph {g}");
                }
            }
            // Skip sets for the flag combinations fonts use.
            for flags in [0x0008u16, 0x0002, 0x0004, 0x0100, 0x0200] {
                for _ in 0..2 {
                    plan.with_skips(font, flags, None, |skips| {
                        for g in (0..glyphs).step_by(7) {
                            assert_eq!(skips(g), font.lookup_skips_glyph(flags, None, g));
                        }
                    });
                }
            }
            // Single substitutions: every type-1 lookup against a
            // spread of glyphs, twice, so memo hits and evictions are
            // both exercised.
            let singles: Vec<u16> = font
                .gsub_lookup_list()
                .into_iter()
                .filter(|&(_, ty, _)| ty == 1)
                .map(|(i, _, _)| i)
                .collect();
            assert!(!singles.is_empty());
            for _ in 0..2 {
                for &lookup in &singles {
                    for g in (0..glyphs).step_by(3) {
                        assert_eq!(
                            plan.single_substitute(font, lookup, g),
                            font.gsub_apply_lookup_type_1(lookup, g),
                            "lookup {lookup} glyph {g}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn stages_and_forms_are_built_once() {
        let [face, _] = fonts();
        let _ = face;
        let plan = LayoutPlan::new();
        let mut built = 0;
        let features = [(*b"liga", 1u32, 1u32)];
        for _ in 0..3 {
            let stage = plan.stage(false, *b"latn", &features, || {
                built += 1;
                Vec::new()
            });
            assert!(stage.is_empty());
        }
        assert_eq!(built, 1);
        // A different request is a different stage.
        plan.stage(true, *b"latn", &features, || {
            built += 1;
            Vec::new()
        });
        plan.stage(false, *b"arab", &features, || {
            built += 1;
            Vec::new()
        });
        assert_eq!(built, 3);

        let mut probed = 0;
        let key = (*b"dev2", *b"half", '\u{915}', '\u{94d}');
        for _ in 0..3 {
            assert!(plan.form(key, || {
                probed += 1;
                true
            }));
        }
        assert_eq!(probed, 1);
    }

    #[test]
    fn kept_script_lists_are_bounded() {
        let [face, _] = fonts();
        let font = face.ttf().unwrap();
        let plan = LayoutPlan::new();
        for i in 0..2000u32 {
            let tag = [b'q', (i >> 16) as u8, (i >> 8) as u8, i as u8];
            assert!(plan.gsub_features(font, tag).is_empty());
        }
        assert_eq!(plan.script_features.lock().unwrap().len(), MAX_KEPT_SCRIPTS);
        let direct = font.gsub_features_for_script(*b"latn", None);
        assert!(!direct.is_empty());
        assert_eq!(plan.gsub_features(font, *b"latn").len(), direct.len());
    }

    fn fonts() -> [crate::Face; 2] {
        [
            crate::Face::from_ttf_bytes(
                include_bytes!("../../tests/fixtures/InterVariable.ttf").to_vec(),
            )
            .unwrap(),
            crate::Face::from_ttf_bytes(
                include_bytes!("../../tests/fixtures/DejaVuSans.ttf").to_vec(),
            )
            .unwrap(),
        ]
    }

    /// Every planned fact equals what the font reports directly, on
    /// the first (filling) and second (kept) query alike.
    #[test]
    fn plan_matches_the_font() {
        for face in fonts() {
            let font = face.ttf().unwrap();
            let plan = LayoutPlan::new();
            for _ in 0..2 {
                let list = font.gsub_lookup_list();
                assert!(!list.is_empty());
                for &(index, ty, _) in &list {
                    assert_eq!(plan.gsub_lookup_type(font, index), Some(ty));
                }
                assert_eq!(plan.gsub_lookup_type(font, list.len() as u16 + 7), None);

                let gpos = font.gpos_lookup_list();
                let planned = plan.gpos(font);
                let of_type = |want: &[u16]| -> Vec<(u16, u16)> {
                    gpos.iter()
                        .filter(|&&(_, ty, _)| want.contains(&ty))
                        .map(|&(i, ty, _)| (i, ty))
                        .collect()
                };
                assert_eq!(
                    planned.single,
                    of_type(&[1]).iter().map(|p| p.0).collect::<Vec<_>>()
                );
                assert_eq!(
                    planned.cursive.iter().map(|c| c.0).collect::<Vec<_>>(),
                    of_type(&[3]).iter().map(|p| p.0).collect::<Vec<_>>()
                );
                for &(index, rtl) in &planned.cursive {
                    assert_eq!(rtl, font.gpos_lookup_flags(index) & 1 != 0);
                }
                assert_eq!(planned.contextual, of_type(&[7, 8]));

                for tag in [*b"latn", *b"cyrl", *b"arab", *b"DFLT", *b"zzzz"] {
                    let direct = font.gsub_features_for_script(tag, None);
                    let kept = plan.gsub_features(font, tag);
                    assert_eq!(kept.len(), direct.len());
                    for (k, d) in kept.iter().zip(&direct) {
                        assert_eq!((k.tag, &k.lookups), (d.tag, &d.lookup_indices));
                    }
                    let liga: Vec<u16> = direct
                        .iter()
                        .filter(|f| &f.tag == b"liga")
                        .flat_map(|f| f.lookup_indices.iter().copied())
                        .collect();
                    assert_eq!(plan.gsub_feature_lookups(font, tag, b"liga"), liga);
                }

                for feature in [b"ccmp", b"calt", b"liga", b"zzzz"] {
                    let direct = GENERAL_SCRIPT_TAGS
                        .iter()
                        .map(|&tag| {
                            font.gsub_features_for_script(tag, None)
                                .iter()
                                .filter(|f| &f.tag == feature)
                                .flat_map(|f| f.lookup_indices.iter().copied())
                                .collect::<Vec<u16>>()
                        })
                        .find(|hits| !hits.is_empty())
                        .unwrap_or_default();
                    assert_eq!(&*plan.general_lookups(font, feature), &direct[..]);
                }
            }
        }
    }
}
