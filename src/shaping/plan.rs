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

use oxideav_ttf::Font;
use std::collections::HashMap;
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
    gpos: OnceLock<GposPlan>,
    ccmp: OnceLock<Vec<u16>>,
    calt: OnceLock<Vec<u16>>,
    /// `GSUB` features of each script's default language system.
    script_features: Mutex<HashMap<[u8; 4], Arc<[PlannedFeature]>>>,
}

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
