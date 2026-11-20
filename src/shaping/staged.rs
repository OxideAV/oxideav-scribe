//! The staged lookup engine: a glyph buffer on which substitution
//! lookups are applied **per glyph and in stages**.
//!
//! The font parser's own engine ([`oxideav_ttf::Font::shape`]) applies
//! one feature set to a whole run. Scripts whose features are chosen
//! per glyph cannot use it: Arabic gives each letter exactly one of
//! `isol` / `init` / `medi` / `fina` (ISO/IEC 14496-22 §6 leaves that
//! choice to the client; the Arabic script-development specification
//! describes the analysis), and the Indic scripts apply their basic
//! features to particular positions of a syllable and reorder glyphs
//! between two groups of features. This module is the client-side
//! engine for those runs:
//!
//! * every glyph carries a **mask** — the set of features that may act
//!   on it;
//! * a plan is a list of **stages**; within a stage the lookups of all
//!   its features run in LookupList order (the layout common-table
//!   rule), and the script may edit the buffer between stages;
//! * the `lookupFlag` skip rules are honoured by matching every
//!   multi-glyph lookup against the *unskipped* glyphs only, and
//!   ligature components are tracked so that marks find their
//!   component in mark-to-ligature positioning.
//!
//! Lookups are applied through the parser's per-lookup entry points.

use crate::shaping::engine::{NO_BASE, RunGlyph};
use crate::shaping::plan::LayoutPlan;
use oxideav_ttf::Font;

/// Mask bit of the features that act on every glyph.
pub(crate) const GLOBAL: u32 = 1;

/// One glyph of the working buffer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Glyph {
    pub(crate) gid: u16,
    pub(crate) cluster: u32,
    /// Features allowed to act on this glyph ([`GLOBAL`] always set).
    pub(crate) mask: u32,
    /// The character the glyph was mapped from (`'\0'` once a lookup
    /// has produced it from something else).
    pub(crate) ch: char,
    /// Non-zero: the ligature this glyph is, or (for a mark) sits in.
    pub(crate) lig_id: u16,
    /// For a mark inside a ligature: the 1-based component it follows.
    pub(crate) lig_comp: u8,
    /// Number of components this glyph stands for (1 unless a
    /// ligature).
    pub(crate) comps: u8,
    /// A default-ignorable character: invisible, zero width.
    pub(crate) ignorable: bool,
    /// Script-specific classification (syllable position and the
    /// like); the engine itself does not read it.
    pub(crate) class: u8,
    /// Script-specific grouping (syllable serial).
    pub(crate) group: u16,
    /// A substitution lookup replaced this glyph.
    pub(crate) substituted: bool,
    /// The glyph is the result of a ligature substitution.
    pub(crate) ligated: bool,
    /// The glyph stands for a combining mark character (general
    /// category M), whatever class the font's GDEF gives the glyph.
    pub(crate) from_mark: bool,
}

impl Glyph {
    pub(crate) fn new(gid: u16, ch: char, cluster: u32) -> Self {
        Self {
            gid,
            cluster,
            mask: GLOBAL,
            ch,
            lig_id: 0,
            lig_comp: 0,
            comps: 1,
            ignorable: false,
            class: 0,
            group: 0,
            substituted: false,
            ligated: false,
            from_mark: crate::shaping::text::is_mark(ch),
        }
    }
}

/// One lookup of a stage.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StageLookup {
    pub(crate) index: u16,
    /// Effective lookup type (extension lookups unwrapped).
    pub(crate) ty: u16,
    pub(crate) flags: u16,
    pub(crate) mark_filtering_set: Option<u16>,
    /// Glyphs must carry one of these mask bits.
    pub(crate) mask: u32,
    /// Alternate to take from an alternate-substitution lookup
    /// (0-based).
    pub(crate) alternate: u16,
}

/// `lookupFlag` bits that make a lookup skip glyphs: IGNORE_BASE_GLYPHS,
/// IGNORE_LIGATURES, IGNORE_MARKS, USE_MARK_FILTERING_SET and the
/// mark-attachment class byte.
const SKIP_FLAGS: u16 = 0xFF1E;

/// How many unskipped glyphs before / after the current one a context
/// lookup is shown. Real backtrack and lookahead sequences are a
/// handful of glyphs; the window keeps the cost of a match independent
/// of the run length.
const CONTEXT_BEFORE: usize = 32;
const CONTEXT_AFTER: usize = 64;

/// How many times its initial length substitution may grow a run.
const MAX_GROWTH: usize = 16;

/// The longest ligature a lookup is offered, in components.
const LIGATURE_COMPONENTS: usize = 24;

/// [`Buffer::slot`] value of a glyph the current lookup skips.
const SKIPPED: u32 = u32::MAX;

/// The features of one stage: `(tag, mask bit, value)`.
pub(crate) type StageFeatures<'a> = &'a [([u8; 4], u32, u32)];

/// Resolve a stage's features to its lookups, in LookupList order.
/// `features` come from the font's script / language-system record;
/// a lookup shared by several features takes the union of their masks.
pub(crate) fn plan_stage(
    font: &Font<'_>,
    plan: &LayoutPlan,
    table_features: &[crate::shaping::plan::PlannedFeature],
    stage: StageFeatures<'_>,
    gpos: bool,
) -> Vec<StageLookup> {
    let mut out: Vec<StageLookup> = Vec::new();
    for &(tag, mask, value) in stage {
        if value == 0 {
            continue;
        }
        for feature in table_features.iter().filter(|f| f.tag == tag) {
            for &index in &feature.lookups {
                if let Some(l) = out.iter_mut().find(|l| l.index == index) {
                    l.mask |= mask;
                    continue;
                }
                let ty = if gpos {
                    plan.gpos_lookup_type(font, index)
                } else {
                    plan.gsub_lookup_type(font, index)
                };
                let Some(ty) = ty else { continue };
                let (flags, mark_filtering_set) = if gpos {
                    (
                        font.gpos_lookup_flags(index),
                        font.gpos_lookup_mark_filtering_set(index),
                    )
                } else {
                    (
                        font.gsub_lookup_flags(index),
                        font.gsub_lookup_mark_filtering_set(index),
                    )
                };
                out.push(StageLookup {
                    index,
                    ty,
                    flags,
                    mark_filtering_set,
                    mask,
                    alternate: (value.saturating_sub(1)).min(u32::from(u16::MAX)) as u16,
                });
            }
        }
    }
    out.sort_by_key(|l| l.index);
    out
}

/// The working buffer plus the bookkeeping the lookups need.
#[derive(Debug, Default)]
pub(crate) struct Buffer {
    pub(crate) glyphs: Vec<Glyph>,
    /// Longest the buffer may become through substitution.
    limit: usize,
    /// Lookups may not match across a change of [`Glyph::group`]
    /// (syllable-by-syllable shaping).
    pub(crate) bounded: bool,
    /// ZERO WIDTH JOINER takes part in substitution matching (and so
    /// keeps the glyphs around it from combining) instead of being
    /// looked through.
    pub(crate) joiners_block: bool,
    next_lig_id: u16,
    /// The current lookup's view (see [`Buffer::prepare`]): glyph ids
    /// of the unskipped glyphs, their buffer indices, and each buffer
    /// position's place in the view.
    view: Vec<u16>,
    view_index: Vec<usize>,
    slot: Vec<u32>,
    /// What the view was built for: the lookup's skip flags, its mark
    /// filtering set, and whether it is a positioning lookup.
    view_key: Option<(u16, Option<u16>, bool)>,
    /// An edit made the view stale.
    dirty: bool,
}

impl Buffer {
    pub(crate) fn new(glyphs: Vec<Glyph>) -> Self {
        Self {
            // One-to-many substitutions may grow a run, but not without
            // bound: a font whose lookups each multiply the glyphs
            // would otherwise grow it exponentially.
            limit: glyphs.len().saturating_mul(MAX_GROWTH).saturating_add(256),
            glyphs,
            bounded: false,
            joiners_block: false,
            next_lig_id: 0,
            view: Vec::new(),
            view_index: Vec::new(),
            slot: Vec::new(),
            view_key: None,
            dirty: true,
        }
    }

    /// The caller reordered or otherwise edited [`Self::glyphs`]
    /// directly: the kept view no longer describes the buffer.
    pub(crate) fn invalidate(&mut self) {
        self.dirty = true;
    }

    fn fresh_lig_id(&mut self) -> u16 {
        self.next_lig_id = self.next_lig_id.wrapping_add(1).max(1);
        self.next_lig_id
    }

    /// Build the lookup's view of the buffer: the glyph ids of every
    /// glyph the lookup does not skip, their buffer indices, and for
    /// each buffer position its place in the view ([`SKIPPED`] when
    /// the lookup passes over it). Rebuilt after every edit.
    ///
    /// Besides the `lookupFlag` rules, an invisible default-ignorable
    /// glyph is transparent to matching — except ZERO WIDTH NON-JOINER
    /// in substitution, whose purpose is to keep its neighbours apart
    /// (and ZERO WIDTH JOINER where the script says it blocks).
    fn prepare(
        &mut self,
        font: &Font<'_>,
        plan: &LayoutPlan,
        lookup: &StageLookup,
        positioning: bool,
    ) {
        // Lookups that skip the same glyphs share a view.
        let key = (
            lookup.flags & SKIP_FLAGS,
            lookup
                .mark_filtering_set
                .filter(|_| lookup.flags & 0x10 != 0),
            positioning,
        );
        if !self.dirty && self.view_key == Some(key) && self.slot.len() == self.glyphs.len() {
            return;
        }
        self.view_key = Some(key);
        self.view.clear();
        self.view_index.clear();
        self.slot.clear();
        let flagged = lookup.flags & SKIP_FLAGS != 0;
        let joiners_block = self.joiners_block;
        let (view, view_index, slot) = (&mut self.view, &mut self.view_index, &mut self.slot);
        let mut fill = |skips: &mut dyn FnMut(u16) -> bool| {
            for (i, g) in self.glyphs.iter().enumerate() {
                let skipped = if g.ignorable {
                    positioning || !(g.ch == '\u{200c}' || (g.ch == '\u{200d}' && joiners_block))
                } else {
                    flagged && skips(g.gid)
                };
                if skipped {
                    slot.push(SKIPPED);
                } else {
                    slot.push(view.len() as u32);
                    view.push(g.gid);
                    view_index.push(i);
                }
            }
        };
        if flagged {
            plan.with_skips(font, lookup.flags, lookup.mark_filtering_set, |skips| {
                fill(skips)
            });
        } else {
            fill(&mut |_| false);
        }
        self.dirty = false;
    }

    /// Give the glyph at `i` a new glyph id, keeping the view in step:
    /// the view takes the id when the new glyph is as visible to the
    /// current lookup as the old one was, and is rebuilt otherwise.
    fn replace(&mut self, font: &Font<'_>, plan: &LayoutPlan, i: usize, gid: u16) {
        self.glyphs[i].gid = gid;
        self.glyphs[i].substituted = true;
        if self.dirty {
            return;
        }
        let (Some((flags, set, _)), Some(&k)) = (self.view_key, self.slot.get(i)) else {
            self.dirty = true;
            return;
        };
        let skipped = !self.glyphs[i].ignorable
            && flags != 0
            && plan.with_skips(font, flags, set, |skips| skips(gid));
        if k != SKIPPED && !skipped {
            self.view[k as usize] = gid;
        } else if (k == SKIPPED) != skipped || self.glyphs[i].ignorable {
            self.dirty = true;
        }
    }

    /// The stretch of the view a lookup is shown for the glyph at
    /// buffer position `at` (which must not be skipped): up to
    /// `before` unskipped glyphs before it and `after` from it on,
    /// within its group when matching is bounded. Returns the view
    /// range and the position of `at` inside it.
    fn window(&self, at: usize, before: usize, after: usize) -> (std::ops::Range<usize>, usize) {
        let k = self.slot[at] as usize;
        let mut lo = k.saturating_sub(before);
        let mut hi = (k + after).min(self.view.len());
        if self.bounded {
            let group = self.glyphs[at].group;
            if let Some(j) = (lo..k)
                .rev()
                .find(|&j| self.glyphs[self.view_index[j]].group != group)
            {
                lo = j + 1;
            }
            if let Some(j) = (k + 1..hi).find(|&j| self.glyphs[self.view_index[j]].group != group) {
                hi = j;
            }
        }
        (lo..hi, k - lo)
    }

    /// Run the substitution lookups of one stage.
    pub(crate) fn substitute(
        &mut self,
        font: &Font<'_>,
        plan: &LayoutPlan,
        lookups: &[StageLookup],
    ) {
        for lookup in lookups {
            if self.glyphs.is_empty() {
                return;
            }
            self.prepare(font, plan, lookup, false);
            match lookup.ty {
                1..=3 => self.apply_single(font, plan, lookup),
                4 => self.apply_ligature(font, plan, lookup),
                5 | 6 => self.apply_context(font, plan, lookup),
                8 => self.apply_reverse(font, plan, lookup),
                _ => {}
            }
        }
    }

    /// The glyph at `i` is one `lookup` may act on: carrying one of
    /// its mask bits and not skipped. Refreshes the view first when an
    /// edit made it stale.
    fn acts_on(
        &mut self,
        font: &Font<'_>,
        plan: &LayoutPlan,
        lookup: &StageLookup,
        i: usize,
    ) -> bool {
        if self.dirty {
            self.prepare(font, plan, lookup, false);
        }
        self.glyphs[i].mask & lookup.mask != 0 && self.slot[i] != SKIPPED
    }

    /// Lookup types 1 (single), 2 (multiple) and 3 (alternate): one
    /// glyph in.
    fn apply_single(&mut self, font: &Font<'_>, plan: &LayoutPlan, lookup: &StageLookup) {
        let mut i = 0;
        while i < self.glyphs.len() {
            if !self.acts_on(font, plan, lookup, i) {
                i += 1;
                continue;
            }
            let g = self.glyphs[i];
            match lookup.ty {
                1 => {
                    if let Some(rep) = plan.single_substitute(font, lookup.index, g.gid) {
                        self.replace(font, plan, i, rep);
                    }
                    i += 1;
                }
                3 => {
                    if let Some(rep) =
                        font.gsub_apply_lookup_type_3(lookup.index, g.gid, lookup.alternate)
                    {
                        self.replace(font, plan, i, rep);
                    }
                    i += 1;
                }
                _ => match font.gsub_apply_lookup_type_2(lookup.index, g.gid) {
                    Some(seq) if seq.len() == 1 => {
                        self.replace(font, plan, i, seq[0]);
                        i += 1;
                    }
                    Some(seq) if !seq.is_empty() && self.glyphs.len() + seq.len() <= self.limit => {
                        let n = seq.len();
                        let expanded = seq.into_iter().map(|gid| Glyph {
                            gid,
                            substituted: true,
                            ..g
                        });
                        self.glyphs.splice(i..=i, expanded);
                        self.dirty = true;
                        i += n;
                    }
                    _ => i += 1,
                },
            }
        }
    }

    /// Lookup type 4: ligature substitution over the unskipped glyphs.
    fn apply_ligature(&mut self, font: &Font<'_>, plan: &LayoutPlan, lookup: &StageLookup) {
        let mut i = 0;
        while i < self.glyphs.len() {
            if !self.acts_on(font, plan, lookup, i) {
                i += 1;
                continue;
            }
            let (range, _) = self.window(i, 0, LIGATURE_COMPONENTS);
            if let Some((rep, n)) =
                font.gsub_apply_lookup_type_4(lookup.index, &self.view[range.clone()])
                && n >= 1
                && n <= range.len()
            {
                let parts: Vec<usize> = self.view_index[range.start..range.start + n].to_vec();
                self.ligate(font, plan, &parts, rep);
            }
            i += 1;
        }
    }

    /// Replace the glyphs at `parts` (ascending buffer indices) by the
    /// single glyph `rep`, keeping the glyphs skipped between them and
    /// recording which component each of those now belongs to.
    fn ligate(&mut self, font: &Font<'_>, plan: &LayoutPlan, parts: &[usize], rep: u16) {
        let first = parts[0];
        if parts.len() == 1 {
            self.replace(font, plan, first, rep);
            return;
        }
        self.dirty = true;
        let last = *parts.last().expect("non-empty");
        let id = self.fresh_lig_id();
        let cluster = self.glyphs[first..=last]
            .iter()
            .map(|g| g.cluster)
            .min()
            .unwrap_or(self.glyphs[first].cluster);
        // Marks after each component (and after the last one) move to
        // the new ligature, numbered by the component they follow.
        let mut offset: u32 = 0;
        for (j, &p) in parts.iter().enumerate() {
            let part = self.glyphs[p];
            let end = parts.get(j + 1).copied().unwrap_or(self.glyphs.len());
            for m in &mut self.glyphs[p + 1..end] {
                if j + 1 == parts.len() && !(m.ignorable || font.is_mark_glyph(m.gid)) {
                    break;
                }
                let comp = if part.lig_id != 0 && m.lig_id == part.lig_id && m.lig_comp > 0 {
                    offset + u32::from(m.lig_comp)
                } else {
                    offset + u32::from(part.comps)
                };
                m.lig_id = id;
                m.lig_comp = comp.min(255) as u8;
                if j + 1 < parts.len() {
                    m.cluster = cluster;
                }
            }
            offset += u32::from(part.comps);
        }
        // The ligature keeps the highest-ranking role of its parts
        // (a conjunct that swallowed the base consonant is the base).
        let class = parts
            .iter()
            .map(|&p| self.glyphs[p].class)
            .find(|&c| c == crate::shaping::indic_ot::BASE)
            .unwrap_or(self.glyphs[first].class);
        let mask = parts.iter().fold(0, |m, &p| m | self.glyphs[p].mask);
        let g = &mut self.glyphs[first];
        g.gid = rep;
        g.class = class;
        g.mask = mask;
        g.cluster = cluster;
        g.comps = offset.min(255) as u8;
        g.lig_id = id;
        g.lig_comp = 0;
        g.substituted = true;
        g.ligated = true;
        g.ch = '\0';
        for &p in parts[1..].iter().rev() {
            self.glyphs.remove(p);
        }
    }

    /// Lookup types 5 and 6: (chained) contextual substitution. The
    /// lookup is shown the unskipped glyphs around the position and
    /// answers with their rewritten sequence; the difference is
    /// applied to the buffer.
    fn apply_context(&mut self, font: &Font<'_>, plan: &LayoutPlan, lookup: &StageLookup) {
        let mut i = 0;
        while i < self.glyphs.len() {
            if !self.acts_on(font, plan, lookup, i) {
                i += 1;
                continue;
            }
            let (range, pos) = self.window(i, CONTEXT_BEFORE, CONTEXT_AFTER);
            let old = &self.view[range.clone()];
            let rewritten = if lookup.ty == 5 {
                font.gsub_apply_lookup_type_5(lookup.index, old, pos)
            } else {
                font.gsub_apply_lookup_type_6(lookup.index, old, pos)
            };
            let Some(new) = rewritten else {
                i += 1;
                continue;
            };
            // The rewritten stretch: what lies between the common
            // prefix and the common suffix. It starts at the matched
            // position at the earliest.
            let mut prefix = pos;
            while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
                prefix += 1;
            }
            let mut suffix = 0;
            while suffix < old.len() - prefix
                && suffix < new.len() - prefix
                && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
            {
                suffix += 1;
            }
            let old_mid = prefix..old.len() - suffix;
            let new_mid: Vec<u16> = new[prefix..new.len() - suffix].to_vec();
            if old_mid.is_empty() && new_mid.is_empty() {
                // Matched without changing anything (an "ignore" rule).
                i += 1;
                continue;
            }
            let window_index = &self.view_index[range.clone()];
            let targets: Vec<usize> = window_index[old_mid].to_vec();
            let insert_at = window_index.get(prefix).copied();
            if targets.len() == new_mid.len() {
                for (&t, &gid) in targets.iter().zip(&new_mid) {
                    self.replace(font, plan, t, gid);
                }
                i = targets.last().map_or(i, |&t| t.max(i)) + 1;
            } else if new_mid.len() == 1 && !targets.is_empty() {
                self.ligate(font, plan, &targets, new_mid[0]);
                self.dirty = true;
                i = targets[0] + 1;
            } else if self.glyphs.len() + new_mid.len() > self.limit {
                // Growth bound reached: leave the glyphs as they are.
                i += 1;
            } else if let Some(&at) = targets.first() {
                // n glyphs become m: the new glyphs take the place of
                // the first, the rest of the old ones go.
                let template = self.glyphs[at];
                for &t in targets[1..].iter().rev() {
                    self.glyphs.remove(t);
                }
                let m = new_mid.len();
                let expanded = new_mid.into_iter().map(|gid| Glyph {
                    gid,
                    substituted: true,
                    ..template
                });
                self.glyphs.splice(at..=at, expanded);
                self.dirty = true;
                i = at + m.max(1);
            } else {
                // Pure insertion before the suffix.
                let at = insert_at.unwrap_or(self.glyphs.len());
                let template = self.glyphs[i];
                let m = new_mid.len();
                let inserted = new_mid.into_iter().map(|gid| Glyph {
                    gid,
                    substituted: true,
                    ..template
                });
                self.glyphs.splice(at..at, inserted);
                self.dirty = true;
                i = at + m;
            }
        }
    }

    /// Lookup type 8: reverse chaining single substitution, processed
    /// from the end of the run.
    fn apply_reverse(&mut self, font: &Font<'_>, plan: &LayoutPlan, lookup: &StageLookup) {
        for i in (0..self.glyphs.len()).rev() {
            if !self.acts_on(font, plan, lookup, i) {
                continue;
            }
            let (range, pos) = self.window(i, CONTEXT_BEFORE, CONTEXT_AFTER);
            if let Some(rep) = font.gsub_apply_lookup_type_8(lookup.index, &self.view[range], pos) {
                self.replace(font, plan, i, rep);
            }
        }
    }

    /// Position the buffer: advances from `hmtx`, then the positioning
    /// lookups in LookupList order. `zero_marks` gives every GDEF mark
    /// a zero advance (the default for scripts whose marks never take
    /// room on the line); attached marks are zeroed regardless.
    pub(crate) fn position(
        &mut self,
        font: &Font<'_>,
        plan: &LayoutPlan,
        lookups: &[StageLookup],
        legacy_kern: bool,
        zero_marks: bool,
        rtl: bool,
    ) -> Vec<RunGlyph> {
        let n = self.glyphs.len();
        let space = font.glyph_index(' ').unwrap_or(0);
        let varies = crate::shaping::engine::instance_varies(font);
        let mut out: Vec<RunGlyph> = self
            .glyphs
            .iter()
            .map(|g| RunGlyph {
                gid: if g.ignorable { space } else { g.gid },
                cluster: g.cluster,
                x_off: 0,
                y_off: 0,
                x_adv: if g.ignorable {
                    0
                } else {
                    crate::shaping::engine::advance_at_instance(font, varies, g.gid)
                },
                y_adv: 0,
                base: NO_BASE,
                adj: 0,
            })
            .collect();
        // GDEF class 3, and — for attachment — any glyph that stands
        // for a mark character (a font may class a ligature of two
        // marks as a ligature and still attach it as a mark).
        let mut gdef_mark: Vec<bool> = Vec::with_capacity(n);
        plan.marks_of(font, self.glyphs.iter().map(|g| g.gid), &mut gdef_mark);
        for (m, g) in gdef_mark.iter_mut().zip(&self.glyphs) {
            *m &= !g.ignorable;
        }
        let is_mark: Vec<bool> = self
            .glyphs
            .iter()
            .zip(&gdef_mark)
            .map(|(g, &m)| m || (g.from_mark && !g.ignorable))
            .collect();
        // Anchor deltas of attached marks, resolved after all lookups.
        let mut attach: Vec<Option<(usize, i32, i32)>> = vec![None; n];
        // Cursive links: (earlier glyph, later glyph, rise, the later
        // glyph keeps its place).
        let mut cursive: Vec<(usize, usize, i32, bool)> = Vec::new();
        let (mut pairs_done, mut base_done, mut stacking) = (false, false, false);

        for lookup in lookups {
            self.prepare(font, plan, lookup, true);
            match lookup.ty {
                1 => {
                    for (i, g) in out.iter_mut().enumerate() {
                        if self.slot[i] == SKIPPED {
                            continue;
                        }
                        if let Some(v) = font.gpos_apply_lookup_type_1_var(lookup.index, g.gid) {
                            g.x_off += i32::from(v.x_placement);
                            g.y_off += i32::from(v.y_placement);
                            g.x_adv += i32::from(v.x_advance);
                        }
                    }
                }
                2 if !pairs_done => {
                    // The parser answers pair adjustment for the
                    // table as a whole, so it is taken once, at the
                    // first active pair lookup.
                    pairs_done = true;
                    let mut prev: Option<usize> = None;
                    for i in 0..n {
                        if self.slot[i] == SKIPPED {
                            continue;
                        }
                        if let Some(p) = prev {
                            let k = i32::from(font.lookup_kerning_var(out[p].gid, out[i].gid));
                            out[p].x_adv += k;
                            out[p].adj += k;
                        }
                        prev = Some(i);
                    }
                }
                3 => {
                    let right_to_left = lookup.flags & 1 != 0;
                    let mut prev: Option<usize> = None;
                    for i in 0..n {
                        if self.slot[i] == SKIPPED {
                            continue;
                        }
                        if let Some(p) = prev {
                            let exit = font
                                .gpos_apply_lookup_type_3(lookup.index, out[p].gid)
                                .and_then(|a| a.exit);
                            let entry = font
                                .gpos_apply_lookup_type_3(lookup.index, out[i].gid)
                                .and_then(|a| a.entry);
                            if let (Some(exit), Some(entry)) = (exit, entry) {
                                let between: i32 = out[p + 1..i].iter().map(|g| g.x_adv).sum();
                                if rtl {
                                    // Right-to-left line: the second
                                    // glyph lies to the left of the
                                    // first, its advance running from
                                    // its origin to its entry anchor,
                                    // which meets the first glyph's
                                    // exit anchor.
                                    out[i].x_adv = i32::from(entry.0) - i32::from(exit.0) - between
                                        + out[i].x_off
                                        - out[p].x_off;
                                } else {
                                    // Left-to-right line: the first
                                    // glyph's advance ends where its
                                    // exit anchor puts the second
                                    // glyph's entry anchor.
                                    out[p].x_adv = i32::from(exit.0) - i32::from(entry.0) - between
                                        + out[p].x_off
                                        - out[i].x_off;
                                }
                                cursive.push((
                                    p,
                                    i,
                                    i32::from(exit.1) - i32::from(entry.1),
                                    right_to_left,
                                ));
                            }
                        }
                        prev = Some(i);
                    }
                }
                4 if !base_done => {
                    base_done = true;
                    let mut base: Option<usize> = None;
                    for i in 0..n {
                        if self.glyphs[i].ignorable {
                            continue;
                        }
                        if !is_mark[i] {
                            base = Some(i);
                            continue;
                        }
                        let Some(b) = base else { continue };
                        if attach[i].is_some() || self.slot[i] == SKIPPED {
                            continue;
                        }
                        if let Some((dx, dy)) = font.lookup_mark_to_base_var(out[b].gid, out[i].gid)
                        {
                            attach[i] = Some((b, i32::from(dx), i32::from(dy)));
                        }
                    }
                }
                5 => {
                    let mut base: Option<usize> = None;
                    for i in 0..n {
                        if self.glyphs[i].ignorable {
                            continue;
                        }
                        if !is_mark[i] {
                            base = Some(i);
                            continue;
                        }
                        let Some(b) = base else { continue };
                        if self.slot[i] == SKIPPED {
                            continue;
                        }
                        let lig = self.glyphs[b];
                        let m = self.glyphs[i];
                        let comp = if lig.lig_id != 0 && m.lig_id == lig.lig_id && m.lig_comp > 0 {
                            u16::from(m.lig_comp.min(lig.comps)) - 1
                        } else {
                            u16::from(lig.comps.max(1)) - 1
                        };
                        // The mark's own component first; a component
                        // with no anchor for the mark's class hands
                        // the mark to the nearest earlier one that has.
                        let anchor = (0..=comp).rev().find_map(|c| {
                            font.gpos_apply_lookup_type_5(lookup.index, out[b].gid, c, out[i].gid)
                        });
                        if let Some((dx, dy)) = anchor {
                            attach[i] = Some((b, i32::from(dx), i32::from(dy)));
                        }
                    }
                }
                // Mark-to-mark is answered for the table as a whole
                // and must win over the base / ligature attachment of
                // the same mark, so it runs after the last lookup.
                6 => stacking = true,
                7 | 8 => {
                    for i in 0..n {
                        if self.slot[i] == SKIPPED {
                            continue;
                        }
                        let (range, pos) = self.window(i, CONTEXT_BEFORE, CONTEXT_AFTER);
                        let shown = &self.view[range.clone()];
                        let records = if lookup.ty == 7 {
                            font.gpos_apply_lookup_type_7(lookup.index, shown, pos)
                        } else {
                            font.gpos_apply_lookup_type_8(lookup.index, shown, pos)
                        };
                        for r in records.unwrap_or_default() {
                            let Some(&t) = self.view_index[range.clone()].get(r.glyph_index) else {
                                continue;
                            };
                            out[t].x_off += i32::from(r.value.x_placement);
                            out[t].y_off += i32::from(r.value.y_placement);
                            out[t].x_adv += i32::from(r.value.x_advance);
                        }
                    }
                }
                _ => {}
            }
        }

        if stacking {
            let mut prev: Option<usize> = None;
            for i in 0..n {
                if self.glyphs[i].ignorable {
                    continue;
                }
                if is_mark[i]
                    && let Some(p) = prev
                    && is_mark[p]
                    && same_component(&self.glyphs[p], &self.glyphs[i])
                    && let Some((dx, dy)) = font.lookup_mark_to_mark_var(out[p].gid, out[i].gid)
                {
                    attach[i] = Some((p, i32::from(dx), i32::from(dy)));
                }
                prev = Some(i);
            }
        }

        if legacy_kern {
            let mut prev: Option<usize> = None;
            for i in 0..n {
                if self.glyphs[i].ignorable {
                    continue;
                }
                if let Some(p) = prev {
                    let k = i32::from(font.lookup_kerning(out[p].gid, out[i].gid));
                    out[p].x_adv += k;
                    out[p].adj += k;
                }
                prev = Some(i);
            }
        }

        // Cursive cross-stream alignment (GPOS lookup type 3): with
        // the RIGHT_TO_LEFT flag clear the later glyph is moved to
        // meet the earlier one, and the adjustment cascades forward;
        // set, the earlier glyph is moved and the chain hangs from its
        // last glyph, so links are resolved back to front.
        for &(p, i, rise, _) in cursive.iter().filter(|c| !c.3) {
            out[i].y_off = out[p].y_off + rise;
        }
        for &(p, i, rise, _) in cursive.iter().rev().filter(|c| c.3) {
            out[p].y_off = out[i].y_off - rise;
        }

        // Resolve mark attachments (bases first: indices ascend).
        for i in 0..n {
            if let Some((b, dx, dy)) = attach[i] {
                // A mark on a mark hangs from that mark's own base.
                let (root, bx, by) = match out[b].base {
                    NO_BASE => (b, out[b].x_off, out[b].y_off),
                    root => (root as usize, out[b].x_off, out[b].y_off),
                };
                out[i].x_off = bx + dx;
                out[i].y_off = by + dy;
                out[i].base = root as u32;
                out[i].adj = 0;
                if gdef_mark[i] {
                    out[i].x_adv = 0;
                }
            } else if gdef_mark[i] {
                if zero_marks {
                    out[i].x_adv = 0;
                }
                out[i].adj = 0;
            }
        }
        out
    }
}

/// Two marks may stack when they sit on the same ligature component
/// (or on no ligature at all).
fn same_component(a: &Glyph, b: &Glyph) -> bool {
    a.lig_id != b.lig_id || a.lig_id == 0 || a.lig_comp == b.lig_comp
}
