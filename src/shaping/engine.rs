//! The shaping engine behind [`crate::FaceChain::shape`] and friends:
//! one run of characters, one font, one script, one direction in; a
//! run of positioned glyphs (font units, Y up) out.
//!
//! The work is split the way the OpenType specification splits it
//! (ISO/IEC 14496-22 §6, "the text-processing client" versus the
//! layout tables):
//!
//! 1. **Client side** (here): per-font normalisation (a base and its
//!    marks become the precomposed character when the font has it, a
//!    precomposed character the font lacks becomes base + marks),
//!    canonical ordering of marks, mirroring in right-to-left runs,
//!    script / language-system selection, the default feature set of
//!    the script, hiding of default-ignorable characters.
//! 2. **Table side**: the font parser's lookup engine applies the
//!    union of the active features' lookups in LookupList order
//!    (`oxideav_ttf::Font::shape`) when every glyph of the run takes
//!    the same features; runs that need per-glyph feature selection
//!    (cursive joining forms, syllable-position features) go through
//!    the staged engine in [`super::staged`].

use crate::shaper::ShapeOptions;
use crate::shaping::plan::LayoutPlan;
use crate::shaping::text;
use intl::unicode::script::Script;
use oxideav_ttf::Font;

/// "No attachment base" marker for [`RunGlyph::base`].
pub(crate) const NO_BASE: u32 = u32::MAX;

/// One character of a run with the cluster (byte offset in the
/// caller's string) it reports under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunChar {
    pub(crate) ch: char,
    pub(crate) cluster: u32,
}

/// One shaped glyph, in font design units with Y up.
///
/// When `base` is not [`NO_BASE`] the glyph is attached to the glyph
/// at that index of the same run (a mark on its base) and `x_off` is
/// measured from the **base's** pen position, so the glyph can be
/// placed for either line direction; otherwise `x_off` is measured
/// from the glyph's own pen position. `y_off` is always absolute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunGlyph {
    pub(crate) gid: u16,
    pub(crate) cluster: u32,
    pub(crate) x_off: i32,
    pub(crate) y_off: i32,
    pub(crate) x_adv: i32,
    pub(crate) y_adv: i32,
    pub(crate) base: u32,
    /// How much positioning lookups changed `x_adv` from the glyph's
    /// own advance (pair kerning, an advance adjustment). A
    /// right-to-left line applies that change on the glyph's left,
    /// where the next glyph in reading order is.
    pub(crate) adj: i32,
}

/// What a run is shaped as.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RunSpec<'a> {
    pub(crate) script: Script,
    pub(crate) rtl: bool,
    pub(crate) opts: &'a ShapeOptions,
}

/// Substitution features that are on unless the caller switches them
/// off (OpenType feature registry: "should be active by default").
const DEFAULT_GSUB: [[u8; 4]; 8] = [
    *b"rvrn", *b"ccmp", *b"locl", *b"rlig", *b"liga", *b"clig", *b"calt", *b"rclt",
];

/// Positioning features that are on by default.
const DEFAULT_GPOS: [[u8; 4]; 7] = [
    *b"abvm", *b"blwm", *b"mark", *b"mkmk", *b"kern", *b"dist", *b"curs",
];

/// The feature tags active for a run: the defaults for its direction
/// plus / minus the caller's settings.
pub(crate) fn active_features(spec: &RunSpec<'_>, extra: &[[u8; 4]]) -> Vec<[u8; 4]> {
    let opts = spec.opts;
    let mut set: Vec<[u8; 4]> = Vec::with_capacity(24);
    if opts.default_substitutions {
        set.extend_from_slice(&DEFAULT_GSUB);
        if spec.rtl {
            set.push(*b"rtla");
        } else {
            set.push(*b"ltra");
            set.push(*b"ltrm");
        }
        for t in extra {
            if !set.contains(t) {
                set.push(*t);
            }
        }
    }
    set.extend_from_slice(&DEFAULT_GPOS);
    if opts.vertical {
        set.push(*b"vert");
        set.retain(|t| t != b"kern");
    }
    for &(tag, value) in &opts.features {
        if value == 0 {
            set.retain(|t| *t != tag);
        } else if !set.contains(&tag) {
            set.push(tag);
        }
    }
    set
}

/// The script tags to look for in a layout table's ScriptList, in
/// preference order: the script's own tags (newest shaping model
/// first), then the default script, then Latin as the last resort.
fn script_candidates(script: Script) -> impl Iterator<Item = [u8; 4]> {
    crate::script::ot_script_tags(script)
        .iter()
        .copied()
        .chain([*b"DFLT", *b"dflt", *b"latn"])
}

/// The ScriptList tag a font registers for `script` in GSUB and in
/// GPOS (each table resolved on its own).
pub(crate) fn pick_script_tags(
    font: &Font<'_>,
    plan: &LayoutPlan,
    script: Script,
) -> (Option<[u8; 4]>, Option<[u8; 4]>) {
    let gsub = script_candidates(script).find(|&t| !plan.gsub_features(font, t).is_empty());
    let gpos = script_candidates(script).find(|&t| !plan.gpos_features(font, t).is_empty());
    (gsub, gpos)
}

/// The script records chosen for a run, with access to their features
/// under a language system.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlainTags {
    pub(crate) gsub: Option<[u8; 4]>,
    pub(crate) gpos: Option<[u8; 4]>,
}

impl PlainTags {
    /// The GSUB features of the run's script under `lang`.
    pub(crate) fn gsub_features(
        &self,
        font: &Font<'_>,
        plan: &LayoutPlan,
        lang: Option<[u8; 4]>,
    ) -> std::sync::Arc<[crate::shaping::plan::PlannedFeature]> {
        match (self.gsub, lang) {
            (None, _) => std::sync::Arc::from(Vec::new()),
            (Some(t), None) => plan.gsub_features(font, t),
            (Some(t), Some(_)) => font
                .gsub_features_for_script_at_instance(t, lang)
                .into_iter()
                .map(|f| crate::shaping::plan::PlannedFeature {
                    tag: f.tag,
                    lookups: f.lookup_indices,
                })
                .collect(),
        }
    }

    /// The GPOS features of the run's script under `lang`.
    pub(crate) fn gpos_features(
        &self,
        font: &Font<'_>,
        plan: &LayoutPlan,
        lang: Option<[u8; 4]>,
    ) -> std::sync::Arc<[crate::shaping::plan::PlannedFeature]> {
        match (self.gpos, lang) {
            (None, _) => std::sync::Arc::from(Vec::new()),
            (Some(t), None) => plan.gpos_features(font, t),
            (Some(t), Some(_)) => font
                .gpos_features_for_script_at_instance(t, lang)
                .into_iter()
                .map(|f| crate::shaping::plan::PlannedFeature {
                    tag: f.tag,
                    lookups: f.lookup_indices,
                })
                .collect(),
        }
    }
}

/// Map a normalised run to its nominal glyphs for the staged engine:
/// one glyph per character, a variation selector folded into its
/// base's `cmap` variation glyph, default-ignorable characters marked
/// invisible. Returns the glyphs and the index of each one's source
/// character.
pub(crate) fn base_glyphs(
    font: &Font<'_>,
    chars: &[RunChar],
) -> (Vec<crate::shaping::staged::Glyph>, Vec<usize>) {
    use crate::shaping::staged::Glyph;
    let mut glyphs: Vec<Glyph> = Vec::with_capacity(chars.len());
    let mut sources: Vec<usize> = Vec::with_capacity(chars.len());
    for (i, rc) in chars.iter().enumerate() {
        if text::is_variation_selector(rc.ch)
            && i > 0
            && sources.last() == Some(&(i - 1))
            && let Some(g) = font.lookup_variation(chars[i - 1].ch, rc.ch)
        {
            if let Some(last) = glyphs.last_mut() {
                last.gid = g;
            }
            continue;
        }
        let mut g = Glyph::new(font.glyph_index(rc.ch).unwrap_or(0), rc.ch, rc.cluster);
        g.ignorable = text::is_default_ignorable(rc.ch);
        glyphs.push(g);
        sources.push(i);
    }
    (glyphs, sources)
}

/// Per-font normalisation of a run (see the module docs). Returns
/// `None` when the run is already in the form the font wants.
pub(crate) fn normalise_for_font(font: &Font<'_>, chars: &[RunChar]) -> Option<Vec<RunChar>> {
    let has = |c: char| font.glyph_index(c).is_some_and(|g| g != 0);
    // Nothing to do unless a character is missing from the font or a
    // combining character is present.
    let needs = chars.iter().any(|rc| {
        let c = rc.ch;
        (c as u32) >= 0x300 && (text::ccc(c) != 0 || is_conjoining(c) || !has(c))
            || ((c as u32) >= 0xC0 && !has(c))
    });
    if !needs {
        return None;
    }

    // 1. Decompose what the font cannot draw as one glyph.
    let mut out: Vec<RunChar> = Vec::with_capacity(chars.len() + 4);
    let mut pieces: Vec<char> = Vec::new();
    for rc in chars {
        if has(rc.ch) || text::is_default_ignorable(rc.ch) {
            out.push(*rc);
            continue;
        }
        pieces.clear();
        if text::decompose(rc.ch, &mut pieces) && has(pieces[0]) {
            out.extend(pieces.iter().map(|&ch| RunChar {
                ch,
                cluster: rc.cluster,
            }));
        } else {
            out.push(*rc);
        }
    }

    // 2. Canonical ordering: each maximal sequence of characters with a
    // non-zero combining class is stably sorted by that class.
    let mut i = 0;
    while i < out.len() {
        if text::ccc(out[i].ch) == 0 {
            i += 1;
            continue;
        }
        let start = i;
        while i < out.len() && text::ccc(out[i].ch) != 0 {
            i += 1;
        }
        if i - start > 1 {
            out[start..i].sort_by_key(|rc| mark_order(rc.ch));
        }
    }

    // 3. Compose towards what the font can draw as one glyph: the
    // canonical composition algorithm, with a composite accepted only
    // when the font has it.
    let mut composed: Vec<RunChar> = Vec::with_capacity(out.len());
    let mut starter: Option<usize> = None;
    let mut last_ccc: u8 = 0;
    for rc in out {
        let class = text::ccc(rc.ch);
        if let Some(s) = starter {
            // A character is blocked from the starter by an intervening
            // character of equal or higher class (or by another
            // starter).
            let adjacent = composed.len() == s + 1;
            let blocked = !adjacent && (last_ccc == 0 || last_ccc >= class);
            if !blocked
                && let Some(c) = text::compose_pair(composed[s].ch, rc.ch)
                && has(c)
            {
                composed[s].ch = c;
                composed[s].cluster = composed[s].cluster.min(rc.cluster);
                continue;
            }
        }
        if class == 0 {
            starter = Some(composed.len());
        }
        last_ccc = class;
        composed.push(rc);
    }
    Some(composed)
}

/// Sort key of a combining mark within its sequence: the canonical
/// combining class, with two script-specific refinements. Marks of
/// different classes commute, so either way the result is canonically
/// equivalent text.
///
/// * Hebrew: the points which modify the letter itself — dagesh (class
///   21), rafe (23), shin dot (24), sin dot (25) — go before the vowel
///   points (classes 10..=20, 22). Keeping the letter-modifying point
///   next to its letter is what lets a font's "letter + dagesh"
///   composition lookup match when a vowel was typed (or canonically
///   ordered) in between.
/// * Arabic: shadda (class 33) goes before the vowel marks (classes
///   27..=32, sukun 34). Fonts stack the vowel mark on the shadda with
///   a mark-to-mark anchor, which needs the shadda to come first.
#[inline]
fn mark_order(c: char) -> u8 {
    match text::ccc(c) {
        k @ (21 | 23 | 24 | 25) => k - 20,
        33 => 26,
        k => k,
    }
}

/// Conjoining Hangul jamo (they compose although their combining class
/// is zero).
#[inline]
fn is_conjoining(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x11FF | 0xA960..=0xA97F | 0xD7B0..=0xD7FF)
}

/// Maximal runs of characters that shape under one OpenType script:
/// `(start, end, script)` over `scripts` (the per-character UAX #24
/// resolution). Adjacent Unicode scripts that share an OpenType script
/// tag (Hiragana and Katakana are both `kana`) form one run, so
/// lookups see across the boundary.
pub(crate) fn script_runs(scripts: &[Script]) -> Vec<(usize, usize, Script)> {
    let tag = |s: Script| crate::script::ot_script_tags(s)[0];
    let mut runs = Vec::new();
    let mut start = 0usize;
    while start < scripts.len() {
        let script = scripts[start];
        let mut end = start + 1;
        while end < scripts.len() && (scripts[end] == script || tag(scripts[end]) == tag(script)) {
            end += 1;
        }
        runs.push((start, end, script));
        start = end;
    }
    runs
}

/// Shape one run with one font.
pub(crate) fn shape_face_run(
    font: &Font<'_>,
    plan: &LayoutPlan,
    chars: &[RunChar],
    spec: &RunSpec<'_>,
) -> Vec<RunGlyph> {
    if chars.is_empty() {
        return Vec::new();
    }
    let normalised = normalise_for_font(font, chars);
    let mut chars: std::borrow::Cow<'_, [RunChar]> = match normalised {
        Some(v) => v.into(),
        None => chars.into(),
    };
    if spec.rtl {
        mirror(font, chars.to_mut());
    }
    let (gsub_tag, gpos_tag) = pick_script_tags(font, plan, spec.script);
    let has_mark_feature = gpos_tag.is_some_and(|t| {
        plan.gpos_features(font, t)
            .iter()
            .any(|f| &f.tag == b"mark")
    });
    if !has_mark_feature && chars.iter().any(|rc| text::is_mark(rc.ch)) {
        compose_presentation_forms(font, chars.to_mut());
    }
    if chars
        .iter()
        .any(|rc| crate::shaping::joining::is_joining(rc.ch))
    {
        let tags = PlainTags {
            gsub: gsub_tag,
            gpos: gpos_tag,
        };
        return crate::shaping::joining::shape_joining(font, plan, &chars, spec, &tags);
    }
    if let Some(script) = crate::shaping::indic_ot::indic_script(spec.script) {
        let tags = PlainTags {
            gsub: gsub_tag,
            gpos: gpos_tag,
        };
        return crate::shaping::indic_ot::shape_indic(font, plan, &chars, spec, script, &tags);
    }
    let features = active_features(spec, &[]);
    // The table engine cannot be used when the run needs something it
    // has no input for (a chosen alternate) or does differently from
    // the staged engine's reading of the GPOS chapter (cursive
    // attachment in the line direction, the ligature component a mark
    // belongs to).
    let gpos_types = gpos_tag.map_or(0, |t| plan.gpos_types_for_script(font, t));
    let tags = PlainTags {
        gsub: gsub_tag,
        gpos: gpos_tag,
    };
    if spec.opts.features.iter().any(|&(_, v)| v > 1) || gpos_types & (1 << 3) != 0 {
        return shape_general_staged(font, plan, &chars, spec, &tags, &features);
    }
    let tag = match (gsub_tag, gpos_tag) {
        (Some(a), Some(b)) if a != b => {
            // The two tables disagree on the script record; the table
            // engine takes one tag for both, so prefer the table that
            // has the script's own record.
            let own = crate::script::ot_script_tags(spec.script);
            if own.contains(&a) || !own.contains(&b) {
                a
            } else {
                b
            }
        }
        (Some(a), _) => a,
        (None, Some(b)) => b,
        (None, None) => *b"DFLT",
    };
    let ctx = PlainCtx {
        font,
        tag,
        lang: spec.opts.language,
        features: &features,
        space: font.glyph_index(' ').unwrap_or(0),
    };
    let kern_on = features.contains(b"kern") && !spec.opts.vertical;
    let gpos_kerns = gpos_tag.is_some_and(|t| {
        plan.gpos_features(font, t)
            .iter()
            .any(|f| &f.tag == b"kern")
    });
    // Legacy `kern` table: used when the font's GPOS offers no `kern`
    // feature for the run's script (the two are alternatives, not
    // layers).
    let legacy_kern = kern_on && !gpos_kerns && font.kern_header_variant().is_some();

    let mut glyphs = shape_segments(&ctx, &chars, kern_on && gpos_kerns);
    if gpos_types & (1 << 5) != 0 && mark_follows_ligature(font, &chars, &glyphs) {
        // Which component of a ligature a mark belongs to is tracked
        // by the staged engine.
        return shape_general_staged(font, plan, &chars, spec, &tags, &features);
    }
    if legacy_kern {
        // Invisible glyphs are looked through.
        let mut prev: Option<usize> = None;
        for i in 0..glyphs.len() {
            if glyphs[i].hidden() {
                continue;
            }
            if let Some(p) = prev {
                let k = i32::from(font.lookup_kerning(glyphs[p].gid, glyphs[i].gid));
                glyphs[p].x_adv += k;
                glyphs[p].adj += k;
            }
            prev = Some(i);
        }
    }
    attach_marks(&ctx, &chars, &mut glyphs);
    if spec.rtl && kern_on && gpos_kerns {
        // The table engine reports final advances; a right-to-left
        // line needs to know which part of a change is pair kerning
        // (it is laid out on the other side of the glyph).
        let mut prev: Option<usize> = None;
        for i in 0..glyphs.len() {
            if glyphs[i].hidden() || glyphs[i].base != NO_BASE || font.is_mark_glyph(glyphs[i].gid)
            {
                continue;
            }
            if let Some(p) = prev
                && glyphs[p].x_adv != i32::from(font.glyph_advance(glyphs[p].gid))
            {
                glyphs[p].adj = i32::from(font.lookup_kerning_var(glyphs[p].gid, glyphs[i].gid));
            }
            prev = Some(i);
        }
    }
    if instance_varies(font) {
        // The table engine measures with the default instance's
        // advances; the run is laid out with the instance's own
        // (`HVAR`).
        for g in glyphs.iter_mut().filter(|g| !g.hidden() && g.x_adv != 0) {
            g.x_adv +=
                i32::from(font.glyph_advance_varied(g.gid)) - i32::from(font.glyph_advance(g.gid));
        }
        vary_positioning(font, &mut glyphs, kern_on && gpos_kerns);
    }
    if spec.opts.vertical {
        make_vertical(font, &mut glyphs);
    }
    glyphs
}

/// Bring pair kerning and mark anchors of a run measured at the
/// default instance to the font's current instance: each value is
/// moved by the difference between its varied and its default form
/// (GPOS value records and anchors carry variation deltas, OpenType
/// "Device and VariationIndex tables").
fn vary_positioning(font: &Font<'_>, glyphs: &mut [RunGlyph], kerning: bool) {
    let mut prev_base: Option<usize> = None;
    let mut prev: Option<usize> = None;
    // Movement already applied to the previous mark (a mark stacked on
    // it moves along).
    let mut carried = (0i32, 0i32);
    for i in 0..glyphs.len() {
        if glyphs[i].hidden() {
            continue;
        }
        if glyphs[i].base == NO_BASE {
            if !font.is_mark_glyph(glyphs[i].gid) {
                if kerning && let Some(p) = prev_base {
                    let (a, b) = (glyphs[p].gid, glyphs[i].gid);
                    let d = i32::from(font.lookup_kerning_var(a, b))
                        - i32::from(font.lookup_kerning(a, b));
                    glyphs[p].x_adv += d;
                }
                prev_base = Some(i);
            }
            prev = Some(i);
            carried = (0, 0);
            continue;
        }
        let b = glyphs[i].base as usize;
        let gid = glyphs[i].gid;
        let stacked = prev
            .filter(|&p| p != b && glyphs[p].base != NO_BASE)
            .and_then(|p| {
                let m = glyphs[p].gid;
                Some((
                    font.lookup_mark_to_mark_var(m, gid)?,
                    font.lookup_mark_to_mark(m, gid)?,
                ))
            });
        let moved = match stacked {
            Some((v, d)) => (
                carried.0 + i32::from(v.0) - i32::from(d.0),
                carried.1 + i32::from(v.1) - i32::from(d.1),
            ),
            None => {
                let base = glyphs[b].gid;
                match (
                    font.lookup_mark_to_base_var(base, gid),
                    font.lookup_mark_to_base(base, gid),
                ) {
                    (Some(v), Some(d)) => (
                        i32::from(v.0) - i32::from(d.0),
                        i32::from(v.1) - i32::from(d.1),
                    ),
                    _ => (0, 0),
                }
            }
        };
        glyphs[i].x_off += moved.0;
        glyphs[i].y_off += moved.1;
        carried = moved;
        prev = Some(i);
    }
}

/// `true` when a variation instance other than the default is
/// installed on `font`.
pub(crate) fn instance_varies(font: &Font<'_>) -> bool {
    font.is_variable() && font.normalised_coords().iter().any(|&c| c != 0.0)
}

/// The advance width of `gid` at the font's current instance.
#[inline]
pub(crate) fn advance_at_instance(font: &Font<'_>, varies: bool, gid: u16) -> i32 {
    if varies {
        i32::from(font.glyph_advance_varied(gid))
    } else {
        i32::from(font.glyph_advance(gid))
    }
}

/// Does a mark glyph follow a glyph that stands for more than one
/// base character (a ligature)?
fn mark_follows_ligature(font: &Font<'_>, chars: &[RunChar], glyphs: &[RunGlyph]) -> bool {
    for i in 1..glyphs.len() {
        if glyphs[i].hidden() || !font.is_mark_glyph(glyphs[i].gid) {
            continue;
        }
        let Some(b) = (0..i)
            .rev()
            .find(|&k| !glyphs[k].hidden() && !font.is_mark_glyph(glyphs[k].gid))
        else {
            continue;
        };
        // The characters from the base glyph's cluster up to the next
        // base glyph's: more than one non-mark means a ligature.
        let from = glyphs[b].cluster;
        let to = glyphs[i + 1..]
            .iter()
            .find(|g| !g.hidden() && !font.is_mark_glyph(g.gid))
            .map_or(u32::MAX, |g| g.cluster);
        let bases = chars
            .iter()
            .filter(|rc| {
                rc.cluster >= from
                    && rc.cluster < to
                    && !text::is_mark(rc.ch)
                    && !text::is_default_ignorable(rc.ch)
            })
            .count();
        if bases > 1 {
            return true;
        }
    }
    false
}

/// A run of a script without per-glyph features on the staged engine:
/// one substitution stage with every active feature, then positioning.
fn shape_general_staged(
    font: &Font<'_>,
    plan: &LayoutPlan,
    chars: &[RunChar],
    spec: &RunSpec<'_>,
    tags: &PlainTags,
    features: &[[u8; 4]],
) -> Vec<RunGlyph> {
    use crate::shaping::staged::{Buffer, GLOBAL, plan_stage};
    let (glyphs, _) = base_glyphs(font, chars);
    let mut buffer = Buffer::new(glyphs);
    let stage: Vec<([u8; 4], u32, u32)> = features
        .iter()
        .map(|&t| (t, GLOBAL, spec.opts.feature(t).unwrap_or(1)))
        .chain(std::iter::once((*b" RQD", GLOBAL, 1)))
        .collect();
    let gsub = tags.gsub_features(font, plan, spec.opts.language);
    buffer.substitute(font, &plan_stage(font, plan, &gsub, &stage, false));
    let gpos = tags.gpos_features(font, plan, spec.opts.language);
    let lookups = plan_stage(font, plan, &gpos, &stage, true);
    let kern_on = features.contains(b"kern") && !spec.opts.vertical;
    let legacy_kern =
        kern_on && !gpos.iter().any(|f| &f.tag == b"kern") && font.kern_header_variant().is_some();
    let mut out = buffer.position(font, &lookups, legacy_kern, true, spec.rtl);
    if spec.opts.vertical {
        make_vertical(font, &mut out);
    }
    out
}

/// Replace characters that have a mirrored counterpart
/// (`Bidi_Mirroring_Glyph`) with it, where the font can draw the
/// counterpart (UAX #9 rule L4).
fn mirror(font: &Font<'_>, chars: &mut [RunChar]) {
    for rc in chars {
        if let Some(m) = crate::bidi::mirrored_glyph(rc.ch)
            && font.glyph_index(m).is_some_and(|g| g != 0)
        {
            rc.ch = m;
        }
    }
}

/// For a font that cannot attach marks (no `mark` positioning feature
/// for the script): fold a letter and the mark after it into the
/// compatibility presentation form that is their canonical equivalent
/// (the Alphabetic Presentation Forms block: a Hebrew letter with
/// dagesh, shin with its dot, …) when the font draws that form. Those
/// forms are excluded from canonical composition, so the normalisation
/// step leaves them apart.
fn compose_presentation_forms(font: &Font<'_>, chars: &mut Vec<RunChar>) {
    let mut i = 0;
    while i + 1 < chars.len() {
        let (a, b) = (chars[i].ch, chars[i + 1].ch);
        if text::is_mark(b)
            && let Some(c) = presentation_composite(a, b)
            && font.glyph_index(c).is_some_and(|g| g != 0)
        {
            chars[i].ch = c;
            chars.remove(i + 1);
            // The composite may take the next mark too (shin with
            // dagesh, then its dot).
            continue;
        }
        i += 1;
    }
}

/// The Alphabetic Presentation Forms character whose canonical
/// decomposition is exactly `a` followed by `b`.
fn presentation_composite(a: char, b: char) -> Option<char> {
    if !matches!(a as u32, 0x5D0..=0x5F2 | 0xFB2A..=0xFB4A) {
        return None;
    }
    let mut parts: Vec<char> = Vec::with_capacity(4);
    let mut whole: Vec<char> = Vec::with_capacity(4);
    if !text::decompose(a, &mut whole) {
        whole.push(a);
    }
    whole.push(b);
    (0xFB1Du32..=0xFB4E).filter_map(char::from_u32).find(|&c| {
        parts.clear();
        text::decompose(c, &mut parts) && parts == whole
    })
}

/// Everything the table engine needs for one call.
struct PlainCtx<'f, 'a> {
    font: &'f Font<'a>,
    tag: [u8; 4],
    lang: Option<[u8; 4]>,
    features: &'f [[u8; 4]],
    /// The glyph an invisible character is drawn with.
    space: u16,
}

impl RunGlyph {
    /// An invisible, zero-width stand-in for a default-ignorable
    /// character.
    fn hidden(&self) -> bool {
        self.adj == HIDDEN
    }
}

/// `RunGlyph::adj` marker of a hidden glyph.
const HIDDEN: i32 = i32::MIN;

/// Width, in font units, of a space character the font has no glyph
/// for, from the character's definition (EN / EM quads and spaces,
/// the n-per-em spaces, figure and punctuation space, the medium
/// mathematical space of 4/18 em, the ideographic space). `None` for
/// anything that is not such a space.
fn fallback_space_width(font: &Font<'_>, c: char) -> Option<i32> {
    let em = i32::from(font.units_per_em());
    let advance_of = |c: char| {
        font.glyph_index(c)
            .filter(|&g| g != 0)
            .map(|g| i32::from(font.glyph_advance(g)))
    };
    Some(match c as u32 {
        0x00A0 => advance_of(' ')?,
        0x2000 | 0x2002 => em / 2,
        0x2001 | 0x2003 | 0x3000 => em,
        0x2004 => (em + 1) / 3,
        0x2005 => em / 4,
        0x2006 => (em + 3) / 6,
        0x2007 => advance_of('0')?,
        0x2008 => advance_of('.')?,
        0x2009 | 0x202F => em / 5,
        0x200A => em / 16,
        0x205F => (em * 4 + 9) / 18,
        _ => return None,
    })
}

/// Shape a normalised run through the table engine, handling the
/// characters the tables never see as themselves: variation selectors
/// (consumed into the base's `cmap` variation glyph), ZERO WIDTH
/// NON-JOINER (a boundary no substitution crosses, but that kerning
/// ignores), ZERO WIDTH JOINER (transparent unless a lookup matches
/// it), and spaces the font lacks.
fn shape_segments(ctx: &PlainCtx<'_, '_>, chars: &[RunChar], kern_across: bool) -> Vec<RunGlyph> {
    let font = ctx.font;
    let special = chars.iter().any(|rc| {
        let c = rc.ch as u32;
        c >= 0xA0
            && (c == 0x200C
                || text::is_variation_selector(rc.ch)
                || (c == 0xA0 || (0x2000..=0x205F).contains(&c) || c == 0x3000)
                    && font.glyph_index(rc.ch).is_none_or(|g| g == 0))
    });
    if !special {
        return shape_joiner_aware(ctx, chars);
    }

    // Rewrite pass: resolve variation selectors and missing spaces.
    enum Piece {
        /// Ordinary character.
        Char(RunChar),
        /// A space stood in for by the space glyph at a given width.
        Space(RunChar, i32),
        /// A glyph chosen by a variation sequence.
        Variant(RunChar, u16),
        /// A boundary (ZERO WIDTH NON-JOINER).
        Break(RunChar),
    }
    let mut pieces: Vec<Piece> = Vec::with_capacity(chars.len());
    for (i, rc) in chars.iter().enumerate() {
        if rc.ch == '\u{200c}' {
            pieces.push(Piece::Break(*rc));
        } else if text::is_variation_selector(rc.ch) && i > 0 {
            let base = chars[i - 1];
            match font.lookup_variation(base.ch, rc.ch) {
                Some(g) if Some(g) == font.glyph_index(base.ch) => {
                    // The default glyph is the registered variant:
                    // the selector is consumed.
                }
                Some(g) if matches!(pieces.last(), Some(Piece::Char(_))) => {
                    pieces.pop();
                    pieces.push(Piece::Variant(base, g));
                }
                _ => pieces.push(Piece::Char(*rc)),
            }
        } else if font.glyph_index(rc.ch).is_none_or(|g| g == 0)
            && let Some(w) = fallback_space_width(font, rc.ch)
        {
            pieces.push(Piece::Space(*rc, w));
        } else {
            pieces.push(Piece::Char(*rc));
        }
    }

    let mut out: Vec<RunGlyph> = Vec::with_capacity(chars.len());
    let mut segment: Vec<RunChar> = Vec::new();
    // Index in `out` of the glyph a following segment kerns against.
    let mut kern_left: Option<usize> = None;
    let mut pending_boundary = false;
    let flush = |segment: &mut Vec<RunChar>,
                 out: &mut Vec<RunGlyph>,
                 kern_left: &mut Option<usize>,
                 pending_boundary: &mut bool| {
        if segment.is_empty() {
            return;
        }
        let shaped = shape_joiner_aware(ctx, segment);
        let first = out.len();
        let shift = first as u32;
        out.extend(shaped.into_iter().map(|mut g| {
            if g.base != NO_BASE {
                g.base += shift;
            }
            g
        }));
        if *pending_boundary
            && kern_across
            && let Some(l) = *kern_left
            && let Some(r) = (first..out.len()).find(|&i| !out[i].hidden())
        {
            let k = i32::from(font.lookup_kerning_var(out[l].gid, out[r].gid));
            out[l].x_adv += k;
        }
        *pending_boundary = false;
        *kern_left = (first..out.len()).rev().find(|&i| !out[i].hidden());
        segment.clear();
    };
    for piece in pieces {
        match piece {
            Piece::Char(rc) => segment.push(rc),
            Piece::Space(rc, width) => {
                // Shaped as an ordinary space, then given its width.
                flush(
                    &mut segment,
                    &mut out,
                    &mut kern_left,
                    &mut pending_boundary,
                );
                let at = out.len();
                segment.push(RunChar {
                    ch: ' ',
                    cluster: rc.cluster,
                });
                flush(
                    &mut segment,
                    &mut out,
                    &mut kern_left,
                    &mut pending_boundary,
                );
                for g in &mut out[at..] {
                    if g.gid == ctx.space {
                        g.x_adv = width;
                        g.adj = 0;
                    }
                }
            }
            Piece::Variant(rc, gid) => {
                flush(
                    &mut segment,
                    &mut out,
                    &mut kern_left,
                    &mut pending_boundary,
                );
                out.push(RunGlyph {
                    gid,
                    cluster: rc.cluster,
                    x_off: 0,
                    y_off: 0,
                    x_adv: i32::from(font.glyph_advance(gid)),
                    y_adv: 0,
                    base: NO_BASE,
                    adj: 0,
                });
                kern_left = None;
            }
            Piece::Break(rc) => {
                flush(
                    &mut segment,
                    &mut out,
                    &mut kern_left,
                    &mut pending_boundary,
                );
                out.push(hidden_glyph(ctx.space, rc.cluster));
                pending_boundary = true;
            }
        }
    }
    flush(
        &mut segment,
        &mut out,
        &mut kern_left,
        &mut pending_boundary,
    );
    out
}

fn hidden_glyph(space: u16, cluster: u32) -> RunGlyph {
    RunGlyph {
        gid: space,
        cluster,
        x_off: 0,
        y_off: 0,
        x_adv: 0,
        y_adv: 0,
        base: NO_BASE,
        adj: HIDDEN,
    }
}

/// The table engine with ZERO WIDTH JOINER treated as the lookups'
/// "skip unless you match it" character: the run is shaped with the
/// joiners in place (so a font whose ligature rules spell them out,
/// as emoji fonts do, matches them), and if one comes back
/// unsubstituted the run is shaped again without those, so that the
/// characters around a joiner the font ignores still interact.
fn shape_joiner_aware(ctx: &PlainCtx<'_, '_>, chars: &[RunChar]) -> Vec<RunGlyph> {
    let (glyphs, sources) = shape_plain(ctx, chars);
    if !chars.iter().any(|rc| rc.ch == '\u{200d}') {
        return glyphs;
    }
    let survived: Vec<usize> = glyphs
        .iter()
        .zip(&sources)
        .filter(|(g, ci)| g.hidden() && chars[**ci].ch == '\u{200d}')
        .map(|(_, &ci)| ci)
        .collect();
    if survived.is_empty() {
        return glyphs;
    }
    let mut kept: Vec<RunChar> = Vec::with_capacity(chars.len());
    let mut kept_index: Vec<usize> = Vec::with_capacity(chars.len());
    for (i, rc) in chars.iter().enumerate() {
        if !survived.contains(&i) {
            kept.push(*rc);
            kept_index.push(i);
        }
    }
    let (reshaped, sources) = shape_plain(ctx, &kept);
    // Put an invisible glyph back for each removed joiner, after the
    // glyphs of the characters before it.
    let mut out: Vec<RunGlyph> = Vec::with_capacity(reshaped.len() + survived.len());
    let mut bases: Vec<u32> = Vec::with_capacity(reshaped.len());
    let mut next = 0usize;
    for (g, ci) in reshaped.iter().zip(&sources) {
        let original = kept_index[*ci];
        while next < survived.len() && survived[next] < original {
            out.push(hidden_glyph(ctx.space, chars[survived[next]].cluster));
            next += 1;
        }
        bases.push(out.len() as u32);
        out.push(*g);
    }
    for &ci in &survived[next..] {
        out.push(hidden_glyph(ctx.space, chars[ci].cluster));
    }
    // Attachment indices follow their glyphs.
    for g in &mut out {
        if g.base != NO_BASE {
            g.base = bases[g.base as usize];
        }
    }
    out
}

/// One call of the table engine: every glyph takes the same features.
/// Returns the glyphs and, for each, the index in `chars` of the
/// character it came from.
fn shape_plain(ctx: &PlainCtx<'_, '_>, chars: &[RunChar]) -> (Vec<RunGlyph>, Vec<usize>) {
    let font = ctx.font;
    let mut text = String::with_capacity(chars.len() * 2);
    // Byte offset of each character in `text`.
    let mut starts: Vec<u32> = Vec::with_capacity(chars.len());
    for rc in chars {
        starts.push(text.len() as u32);
        text.push(rc.ch);
    }
    let shaped = font.shape(&text, ctx.tag, ctx.lang, ctx.features);
    let mut out = Vec::with_capacity(shaped.len());
    let mut sources = Vec::with_capacity(shaped.len());
    let ascii = text.len() == chars.len();
    for g in shaped {
        let ci = if ascii {
            g.cluster as usize
        } else {
            starts
                .partition_point(|&s| s <= g.cluster)
                .saturating_sub(1)
        }
        .min(chars.len() - 1);
        let rc = chars[ci];
        sources.push(ci);
        // A default-ignorable character that no lookup turned into
        // something else draws nothing and takes no space.
        if text::is_default_ignorable(rc.ch) && g.glyph_id == font.glyph_index(rc.ch).unwrap_or(0) {
            out.push(hidden_glyph(ctx.space, rc.cluster));
            continue;
        }
        out.push(RunGlyph {
            gid: g.glyph_id,
            cluster: rc.cluster,
            x_off: g.x_offset,
            y_off: g.y_offset,
            x_adv: g.x_advance,
            y_adv: g.y_advance,
            base: NO_BASE,
            adj: 0,
        });
    }
    (out, sources)
}

/// Mark bookkeeping after the table engine: record which glyph each
/// attached mark hangs from (so the run can be laid out in either
/// direction), give marks no advance of their own, and settle the
/// marks that follow another mark.
///
/// For such a mark the engine reports either its attachment to the
/// base or a mark-to-mark adjustment — the latter as a bare delta
/// from the mark below, without that mark's own displacement. The two
/// are told apart by asking the engine where the mark goes on the base
/// alone: an answer that differs from what it reported in context
/// means the mark was stacked, and it is placed from the mark below.
/// (Mark-to-mark positioning thus always builds on mark-to-base — the
/// order "`mark`, then `mkmk`" of the standard-scripts specification —
/// whatever order the two lookups have in the font.)
fn attach_marks(ctx: &PlainCtx<'_, '_>, chars: &[RunChar], glyphs: &mut [RunGlyph]) {
    let font = ctx.font;
    let mut last_base: Option<usize> = None;
    let mut prev: Option<usize> = None;
    for i in 0..glyphs.len() {
        if glyphs[i].hidden() {
            continue;
        }
        let before = prev.replace(i);
        if !font.is_mark_glyph(glyphs[i].gid) {
            last_base = Some(i);
            continue;
        }
        glyphs[i].adj = 0;
        let Some(b) = last_base else {
            glyphs[i].x_adv = 0;
            continue;
        };
        let between: i32 = glyphs[b..i].iter().map(|g| g.x_adv).sum();
        let positioned = glyphs[i].x_adv == 0 && (glyphs[i].x_off != 0 || glyphs[i].y_off != 0);
        let below = before.filter(|&p| p > b);
        if positioned && below.is_none() {
            // On the base: re-express the offset from the base's pen.
            glyphs[i].x_off += between;
            glyphs[i].base = b as u32;
        } else if positioned && let Some(p) = below {
            let on_base = (glyphs[i].x_off + between, glyphs[i].y_off);
            match base_anchor_delta(ctx, chars, &glyphs[b], &glyphs[i]) {
                Probe::Attached(dx, dy) if (dx, dy) != on_base => {
                    stack_on(glyphs, b, p, i);
                }
                Probe::Unattached => stack_on(glyphs, b, p, i),
                _ => {
                    glyphs[i].x_off = on_base.0;
                    glyphs[i].base = b as u32;
                }
            }
        } else if below.is_some() {
            // A mark after other marks that the engine left alone.
            let anchor = match base_anchor_delta(ctx, chars, &glyphs[b], &glyphs[i]) {
                Probe::Attached(dx, dy) => Some((dx, dy)),
                Probe::Unattached => None,
                // The pair shapes differently on its own; ask the
                // font's mark-to-base data directly.
                Probe::Unknown => font
                    .lookup_mark_to_base_var(glyphs[b].gid, glyphs[i].gid)
                    .map(|(dx, dy)| {
                        (
                            i32::from(dx) + glyphs[b].x_off,
                            i32::from(dy) + glyphs[b].y_off,
                        )
                    }),
            };
            if let Some((dx, dy)) = anchor {
                glyphs[i].x_off = dx;
                glyphs[i].y_off = dy;
                glyphs[i].base = b as u32;
            }
        }
        // A mark takes no room on the line (GDEF class 3).
        glyphs[i].x_adv = 0;
    }
}

/// Place mark `i`, whose offsets are a delta from the mark `p` below
/// it, from where `p` actually sits on base `b`.
fn stack_on(glyphs: &mut [RunGlyph], b: usize, p: usize, i: usize) {
    let below_x = if glyphs[p].base == NO_BASE {
        glyphs[p].x_off + glyphs[b..p].iter().map(|g| g.x_adv).sum::<i32>()
    } else {
        glyphs[p].x_off
    };
    glyphs[i].x_off += below_x;
    glyphs[i].y_off += glyphs[p].y_off;
    glyphs[i].base = b as u32;
}

/// What shaping a base and a mark on their own says about the pair.
enum Probe {
    /// The active lookups attach the mark at `(dx, dy)` from the
    /// base's pen position.
    Attached(i32, i32),
    /// They leave it alone.
    Unattached,
    /// The two glyphs are not what the pair shapes to in isolation.
    Unknown,
}

/// Where the active lookups put `mark` on `base` when nothing stands
/// between them. Asked of the table engine itself, by shaping the two
/// characters on their own, so only the run's active mark-to-base
/// lookups answer.
fn base_anchor_delta(
    ctx: &PlainCtx<'_, '_>,
    chars: &[RunChar],
    base: &RunGlyph,
    mark: &RunGlyph,
) -> Probe {
    let font = ctx.font;
    let source = |g: &RunGlyph| {
        chars
            .iter()
            .find(|rc| rc.cluster == g.cluster && font.glyph_index(rc.ch) == Some(g.gid))
            .map(|rc| rc.ch)
    };
    let (Some(b), Some(m)) = (source(base), source(mark)) else {
        return Probe::Unknown;
    };
    let pair: String = [b, m].into_iter().collect();
    let shaped = font.shape(&pair, ctx.tag, ctx.lang, ctx.features);
    match shaped.as_slice() {
        [b, m] if b.glyph_id == base.gid && m.glyph_id == mark.gid => {
            if m.x_advance == 0 && (m.x_offset != 0 || m.y_offset != 0) {
                Probe::Attached(
                    m.x_offset + b.x_advance + (base.x_off - b.x_offset),
                    m.y_offset + (base.y_off - b.y_offset),
                )
            } else {
                Probe::Unattached
            }
        }
        _ => Probe::Unknown,
    }
}

/// Turn a horizontally measured run into a top-to-bottom one: each
/// glyph advances down by its vertical advance and is drawn relative
/// to its vertical origin (`vmtx` / `VORG`; ISO/IEC 14496-22 §5.7.9,
/// §5.4.5), which sits at the middle of the horizontal advance.
fn make_vertical(font: &Font<'_>, glyphs: &mut [RunGlyph]) {
    let em = i32::from(font.units_per_em());
    let ascent = i32::from(font.ascent());
    let varies = instance_varies(font);
    for g in glyphs {
        if g.hidden() {
            continue;
        }
        let h_adv = advance_at_instance(font, varies, g.gid);
        let v_adv = font
            .glyph_advance_height(g.gid)
            .map(i32::from)
            .unwrap_or(em);
        let origin_y = font
            .glyph_vertical_origin_y(g.gid)
            .map(i32::from)
            .unwrap_or(ascent);
        g.x_off -= h_adv / 2;
        g.y_off -= origin_y;
        g.x_adv = 0;
        g.y_adv = -v_adv;
        g.adj = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> Vec<RunChar> {
        text.char_indices()
            .map(|(i, ch)| RunChar {
                ch,
                cluster: i as u32,
            })
            .collect()
    }

    fn string(chars: &[RunChar]) -> String {
        chars.iter().map(|rc| rc.ch).collect()
    }

    #[test]
    fn normalisation_follows_the_font() {
        let face = crate::Face::from_ttf_bytes(
            include_bytes!("../../tests/fixtures/DejaVuSans.ttf").to_vec(),
        )
        .unwrap();
        let font = face.ttf().unwrap();
        // Plain ASCII: untouched.
        assert!(normalise_for_font(font, &run("plain")).is_none());
        // Base + mark composes to the precomposed character the font
        // has; the composite keeps the base's cluster.
        let n = normalise_for_font(font, &run("e\u{301}x")).unwrap();
        assert_eq!(string(&n), "\u{e9}x");
        assert_eq!(n[0].cluster, 0);
        assert_eq!(n[1].cluster, 3);
        // Marks are put in canonical order before composing: the dot
        // below (class 220) sorts before the circumflex (class 230).
        let n = normalise_for_font(font, &run("e\u{302}\u{323}")).unwrap();
        assert_eq!(string(&n), "\u{1ec7}");
        // A pair with no composite stays as it is.
        let n = normalise_for_font(font, &run("q\u{301}")).unwrap();
        assert_eq!(string(&n), "q\u{301}");
    }

    #[test]
    fn default_feature_sets() {
        let opts = ShapeOptions::new();
        let spec = RunSpec {
            script: Script::Latin,
            rtl: false,
            opts: &opts,
        };
        let f = active_features(&spec, &[]);
        for t in [
            b"ccmp", b"liga", b"kern", b"mark", b"mkmk", b"calt", b"ltra",
        ] {
            assert!(f.contains(t), "{:?}", std::str::from_utf8(t));
        }
        assert!(!f.contains(b"smcp") && !f.contains(b"rtla"));
        let opts = ShapeOptions::new()
            .with_feature(*b"liga", false)
            .with_feature(*b"smcp", true);
        let spec = RunSpec {
            script: Script::Arabic,
            rtl: true,
            opts: &opts,
        };
        let f = active_features(&spec, &[*b"init"]);
        assert!(!f.contains(b"liga") && f.contains(b"smcp") && f.contains(b"init"));
        assert!(f.contains(b"rtla") && !f.contains(b"ltra"));
    }
}
