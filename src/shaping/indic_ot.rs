//! The Indic shaping procedure on the staged engine, for the nine
//! scripts whose script-development specifications are staged
//! (Devanagari, Bengali, Gurmukhi, Gujarati, Oriya, Tamil, Telugu,
//! Kannada, Malayalam).
//!
//! The stages are the ones those specifications give ("Shaping
//! Engine": analyse, reorder, shape, position):
//!
//! 1. **Analyse** — the run is cut into syllables by the patterns of
//!    "Find next syllable in the input" (consonant syllable,
//!    vowel-based syllable, stand-alone cluster).
//! 2. **Reorder characters** — the base consonant is found ("starting
//!    from the end of the syllable, move backwards until a consonant
//!    is found that does not have a below-base or post-base form…"),
//!    the consonants' forms being read from the font by trying the
//!    form feature on the two-glyph sequence; matras and signs move to
//!    the positions of the script's reordering-class table.
//! 3. **Basic shaping forms** — `locl`, `nukt`, `akhn`, `rphf`, `rkrf`,
//!    `pref`, `blwf`, `abvf`, `half`, `pstf`, `vatu`, `cjct`, one at a
//!    time, each form feature only on the part of the syllable it
//!    belongs to.
//! 4. **Final reordering** — a pre-base matra moves next to the main
//!    consonant when half forms did not form, the reph moves to the
//!    script's reph position, a pre-base-reordering consonant moves in
//!    front of the base.
//! 5. **Presentation forms** — `init`, `pres`, `abvs`, `blws`, `psts`,
//!    `haln`, `calt` (and the caller's features) together.
//! 6. **Positioning** — `kern`, `dist`, `abvm`, `blwm` (and `mark` /
//!    `mkmk`).
//!
//! Lookups never match across a syllable boundary.

use crate::shaping::engine::{PlainTags, RunChar, RunGlyph, RunSpec, active_features};
use crate::shaping::plan::{LayoutPlan, PlannedFeature};
use crate::shaping::staged::{Buffer, GLOBAL, Glyph, plan_stage};
use crate::shaping::text;
use intl::unicode::script::Script;
use intl::unicode::{IndicPositionalCategory as Ipc, IndicSyllabicCategory as Isc};
use oxideav_ttf::Font;

// Positions inside a syllable, in display order (the specification's
// "Reph + HalfConsonant(s) + MainConsonant(s) + BelowBaseConsonant(s)
// + PostBaseConsonant(s) + MatrasAndSigns", with the matra reordering
// classes slotted in).
const REPH: u8 = 1;
const BEFORE_HALF: u8 = 2;
const PRE_C: u8 = 3;
const BEFORE_MAIN: u8 = 4;
pub(crate) const BASE: u8 = 5;
const AFTER_MAIN: u8 = 6;
const BEFORE_SUB: u8 = 7;
const BELOW_C: u8 = 8;
const AFTER_SUB: u8 = 9;
const BEFORE_POST: u8 = 10;
const POST_C: u8 = 11;
const AFTER_POST: u8 = 12;
const SMVD: u8 = 13;
/// A consonant the `pref` feature turned into its pre-base form.
const PREF_FORMED: u8 = 14;

const RPHF: u32 = 1 << 1;
const PREF: u32 = 1 << 2;
const BLWF: u32 = 1 << 3;
const ABVF: u32 = 1 << 4;
const HALF: u32 = 1 << 5;
const PSTF: u32 = 1 << 6;
const INIT: u32 = 1 << 7;

/// What the shaping procedure needs to know about one script.
#[derive(Debug)]
pub(crate) struct IndicScript {
    /// The consonant(s) that can form a reph.
    ra: &'static [char],
    /// Reordering class of the reph (the script's table).
    reph: u8,
    /// Reordering class of the listed matras and signs; `None` falls
    /// back on the character's `Indic_Positional_Category`.
    class: fn(char) -> Option<u8>,
    /// Where a left-side matra goes when the table does not list it.
    pre_matra: u8,
}

fn devanagari_class(c: char) -> Option<u8> {
    Some(match c as u32 {
        0x093F => BEFORE_HALF,
        0x093E | 0x0940..=0x094C | 0x0962 | 0x0963 => AFTER_SUB,
        _ => return None,
    })
}

fn bengali_class(c: char) -> Option<u8> {
    Some(match c as u32 {
        0x09BF | 0x09C7 | 0x09C8 => BEFORE_HALF,
        0x09C1..=0x09C4 | 0x09E2 | 0x09E3 => AFTER_SUB,
        0x09BE | 0x09C0 | 0x09D7 | 0x0981 => AFTER_POST,
        _ => return None,
    })
}

fn gurmukhi_class(c: char) -> Option<u8> {
    Some(match c as u32 {
        0x0A3F => BEFORE_HALF,
        0x0A47 | 0x0A48 | 0x0A4B | 0x0A4C => AFTER_SUB,
        0x0A41 | 0x0A42 | 0x0A3E | 0x0A40 | 0x0A01 | 0x0A02 | 0x0A70 | 0x0A71 => AFTER_POST,
        _ => return None,
    })
}

fn gujarati_class(c: char) -> Option<u8> {
    Some(match c as u32 {
        0x0ABF => BEFORE_HALF,
        0x0AC5 | 0x0AC7 | 0x0AC8 => AFTER_SUB,
        0x0AC1..=0x0AC4 | 0x0AE2 | 0x0AE3 | 0x0ABE | 0x0AC0 | 0x0AC9 | 0x0ACB | 0x0ACC => {
            AFTER_POST
        }
        _ => return None,
    })
}

fn oriya_class(c: char) -> Option<u8> {
    Some(match c as u32 {
        0x0B47 => BEFORE_HALF,
        0x0B3F | 0x0B56 => AFTER_MAIN,
        0x0B41..=0x0B43 => AFTER_SUB,
        0x0B3E | 0x0B40 | 0x0B57 => AFTER_POST,
        0x0B01 => BEFORE_SUB,
        _ => return None,
    })
}

fn tamil_class(c: char) -> Option<u8> {
    Some(match c as u32 {
        0x0BC6..=0x0BC8 => BEFORE_MAIN,
        0x0BC0 => AFTER_SUB,
        0x0BBF | 0x0BC1 | 0x0BC2 | 0x0BBE | 0x0BD7 => AFTER_POST,
        _ => return None,
    })
}

fn telugu_class(c: char) -> Option<u8> {
    Some(match c as u32 {
        0x0C3E..=0x0C42 | 0x0C46 | 0x0C47 | 0x0C4A..=0x0C4C | 0x0C55 | 0x0C56 => BEFORE_SUB,
        0x0C43 | 0x0C44 => AFTER_SUB,
        _ => return None,
    })
}

fn kannada_class(c: char) -> Option<u8> {
    Some(match c as u32 {
        0x0CBF | 0x0CC6 | 0x0CCC | 0x0CBE | 0x0CE2 | 0x0CE3 | 0x0CC1 | 0x0CC2 => BEFORE_SUB,
        0x0CC3 | 0x0CC4 | 0x0CD5 | 0x0CD6 => AFTER_SUB,
        _ => return None,
    })
}

fn malayalam_class(c: char) -> Option<u8> {
    Some(match c as u32 {
        0x0D46..=0x0D48 => BEFORE_MAIN,
        0x0D3E..=0x0D43 | 0x0D57 => AFTER_POST,
        _ => return None,
    })
}

/// The shaping parameters of `script`, when it is one of the scripts
/// this module covers.
pub(crate) fn indic_script(script: Script) -> Option<&'static IndicScript> {
    macro_rules! cfg {
        ($ra:expr, $reph:expr, $class:expr, $pre:expr) => {
            const {
                &IndicScript {
                    ra: $ra,
                    reph: $reph,
                    class: $class,
                    pre_matra: $pre,
                }
            }
        };
    }
    Some(match script {
        Script::Devanagari => cfg!(&['\u{930}'], BEFORE_POST, devanagari_class, BEFORE_HALF),
        Script::Bengali => cfg!(
            &['\u{9b0}', '\u{9f0}'],
            AFTER_SUB,
            bengali_class,
            BEFORE_HALF
        ),
        Script::Gurmukhi => cfg!(&['\u{a30}'], BEFORE_SUB, gurmukhi_class, BEFORE_HALF),
        Script::Gujarati => cfg!(&['\u{ab0}'], BEFORE_POST, gujarati_class, BEFORE_HALF),
        Script::Oriya => cfg!(&['\u{b30}'], AFTER_MAIN, oriya_class, BEFORE_HALF),
        Script::Tamil => cfg!(&['\u{bb0}'], AFTER_POST, tamil_class, BEFORE_MAIN),
        Script::Telugu => cfg!(&['\u{c30}'], AFTER_POST, telugu_class, BEFORE_HALF),
        Script::Kannada => cfg!(&['\u{cb0}'], AFTER_POST, kannada_class, BEFORE_HALF),
        Script::Malayalam => cfg!(&['\u{d30}'], AFTER_MAIN, malayalam_class, BEFORE_MAIN),
        _ => return None,
    })
}

/// Character categories of the syllable patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cat {
    /// Consonant (`C`).
    C,
    /// Independent vowel (`V`).
    V,
    /// Nukta (`N`).
    N,
    /// Halant / virama (`H`).
    H,
    Zwj,
    Zwnj,
    /// Matra (`M`).
    M,
    /// Syllable modifier sign (`SM`).
    Sm,
    /// Vedic sign, anudatta, other combining mark (`VD`, `A`).
    Vd,
    /// NO-BREAK SPACE, dotted circle, consonant placeholder (`NBSP`).
    Placeholder,
    /// Anything that is not part of a syllable.
    Other,
}

fn category(c: char) -> Cat {
    match c {
        '\u{200d}' => return Cat::Zwj,
        '\u{200c}' => return Cat::Zwnj,
        '\u{a0}' | '\u{25cc}' => return Cat::Placeholder,
        _ => {}
    }
    if (c as u32) < 0x900 {
        return Cat::Other;
    }
    match intl::unicode::indic_syllabic_category(c) {
        Isc::Consonant | Isc::ConsonantDead => Cat::C,
        Isc::ConsonantPlaceholder => Cat::Placeholder,
        Isc::VowelIndependent => Cat::V,
        Isc::Nukta => Cat::N,
        Isc::Virama => Cat::H,
        Isc::VowelDependent => Cat::M,
        Isc::Bindu | Isc::Visarga => Cat::Sm,
        Isc::CantillationMark | Isc::ToneMark | Isc::SyllableModifier => Cat::Vd,
        _ if text::is_mark(c) => Cat::Vd,
        _ => Cat::Other,
    }
}

/// End of the syllable starting at `start` (exclusive), by the three
/// patterns of the specification, and whether it is a well-formed
/// syllable (as opposed to a lone character).
///
/// ```text
/// consonant:  {C+[N]+<H+[<ZWNJ|ZWJ>]|<ZWNJ|ZWJ>+H>} + C+[N]+[A]
///             + [< H+[<ZWNJ|ZWJ>] | {M}+[N]+[H]>] + [SM] + [(VD)]
/// vowel:      [Ra+H]+V+[N]+[<[<ZWJ|ZWNJ>]+H+C|ZWJ+C>]
///             + [{M}+[N]+[H]] + [SM] + [(VD)]
/// standalone: the vowel pattern with NBSP in place of V
/// ```
fn syllable_end(cats: &[Cat], start: usize) -> (usize, bool) {
    let at = |i: usize| cats.get(i).copied().unwrap_or(Cat::Other);
    let joiner = |i: usize| matches!(at(i), Cat::Zwj | Cat::Zwnj);
    let mut i = start;
    // Optional Ra + H in front of a vowel or placeholder.
    let vowel_start =
        if at(i) == Cat::C && at(i + 1) == Cat::H && matches!(at(i + 2), Cat::V | Cat::Placeholder)
        {
            i + 2
        } else {
            i
        };
    if matches!(at(vowel_start), Cat::V | Cat::Placeholder) {
        i = vowel_start + 1;
        if at(i) == Cat::N {
            i += 1;
        }
        // [<[<ZWJ|ZWNJ>]+H+C | ZWJ+C>]
        let j = if joiner(i) { i + 1 } else { i };
        if at(j) == Cat::H && at(j + 1) == Cat::C {
            i = j + 2;
        } else if at(i) == Cat::Zwj && at(i + 1) == Cat::C {
            i += 2;
        }
        if at(i) == Cat::N {
            i += 1;
        }
    } else if at(i) == Cat::C {
        loop {
            // C + [N]
            i += 1;
            if at(i) == Cat::N {
                i += 1;
            }
            // <H+[<ZWNJ|ZWJ>] | <ZWNJ|ZWJ>+H> followed by a consonant
            // continues the conjunct.
            let next = if at(i) == Cat::H {
                Some(if joiner(i + 1) { i + 2 } else { i + 1 })
            } else if joiner(i) && at(i + 1) == Cat::H {
                Some(i + 2)
            } else {
                None
            };
            match next {
                Some(n) if at(n) == Cat::C => i = n,
                _ => break,
            }
        }
        if at(i) == Cat::Vd && cats.get(i).is_some() {
            // [A]: taken with the trailing signs below.
        }
        // A final halant, with its optional joiner.
        if at(i) == Cat::H {
            i += 1;
            if joiner(i) {
                i += 1;
            }
            return (trailing_signs(cats, i), true);
        }
    } else {
        return (start + 1, false);
    }
    // {M} + [N] + [H]
    while at(i) == Cat::M {
        i += 1;
        if at(i) == Cat::N {
            i += 1;
        }
    }
    if at(i) == Cat::H {
        i += 1;
    }
    (trailing_signs(cats, i), true)
}

/// `[SM] + [(VD)]` — taken greedily.
fn trailing_signs(cats: &[Cat], mut i: usize) -> usize {
    while matches!(cats.get(i), Some(Cat::Sm | Cat::Vd)) {
        i += 1;
    }
    i
}

/// One character of a syllable during reordering.
#[derive(Debug, Clone, Copy)]
struct Item {
    ch: char,
    cluster: u32,
    cat: Cat,
    pos: u8,
    mask: u32,
}

/// Font-dependent facts, read by trying a feature on two glyphs.
struct Forms<'f, 'a> {
    font: &'f Font<'a>,
    plan: &'f LayoutPlan,
    features: &'f [PlannedFeature],
    /// The font follows the older model: every form feature is
    /// written for Consonant + Halant.
    old_model: bool,
    /// The script tag answers are kept under (`None`: a language
    /// system or a variation instance may change them).
    keep: Option<[u8; 4]>,
}

impl Forms<'_, '_> {
    /// Does `feature` substitute the two-character sequence `a b` on
    /// its own ("if these two glyphs form a ligature, with no
    /// additional glyphs in context, the consonant has the
    /// corresponding form")?
    fn substitutes(&mut self, feature: [u8; 4], a: char, b: char) -> bool {
        let (font, plan, features) = (self.font, self.plan, self.features);
        let probe = || {
            let lookups = plan_stage(
                font,
                plan,
                features,
                &[(feature, GLOBAL, 1), (*b"locl", GLOBAL, 1)],
                false,
            );
            !lookups.is_empty() && {
                let glyph = |c: char| Glyph::new(font.glyph_index(c).unwrap_or(0), c, 0);
                let mut buffer = Buffer::new(vec![glyph(a), glyph(b)]);
                buffer.substitute(font, plan, &lookups);
                buffer.glyphs.iter().any(|g| g.substituted)
            }
        };
        match self.keep {
            // The answer depends on the font only: kept per face.
            Some(tag) => plan.form((tag, feature, a, b), probe),
            None => probe(),
        }
    }

    /// Does the consonant `c` take the form of `feature` after a
    /// halant `h`?
    fn joined_form(&mut self, feature: [u8; 4], h: char, c: char) -> bool {
        if self.old_model {
            self.substitutes(feature, c, h)
        } else {
            self.substitutes(feature, h, c)
        }
    }
}

/// Reordering class of a matra or sign.
fn sign_class(script: &IndicScript, c: char, cat: Cat) -> u8 {
    if let Some(class) = (script.class)(c) {
        return class;
    }
    if cat != Cat::M {
        return SMVD;
    }
    match intl::unicode::indic_positional_category(c) {
        Ipc::Left | Ipc::VisualOrderLeft => script.pre_matra,
        Ipc::Right => AFTER_POST,
        _ => AFTER_SUB,
    }
}

/// "Reorder characters" for one syllable: find the base, classify the
/// other consonants, give every character its position and the form
/// features that may act on it, and put the syllable in display order.
fn reorder_syllable(script: &IndicScript, forms: &mut Forms<'_, '_>, items: &mut [Item]) {
    let n = items.len();
    let is_joiner = |it: &Item| matches!(it.cat, Cat::Zwj | Cat::Zwnj);
    let consonants: Vec<usize> = (0..n).filter(|&i| items[i].cat == Cat::C).collect();
    let vowel = (0..n).find(|&i| matches!(items[i].cat, Cat::V | Cat::Placeholder));

    // Reph: the syllable starts with Ra + Halant (no joiner after
    // it), has something for the reph to sit on, and the font has a
    // reph form.
    let has_reph = n >= 3
        && script.ra.contains(&items[0].ch)
        && items[1].cat == Cat::H
        && !is_joiner(&items[2])
        && (consonants.len() > 1 || vowel.is_some())
        && forms.substitutes(*b"rphf", items[0].ch, items[1].ch);

    // The halant joining consonant `i` to what precedes it.
    let halant_before = |items: &[Item], i: usize| -> Option<usize> {
        let mut k = i.checked_sub(1)?;
        if is_joiner(&items[k]) {
            k = k.checked_sub(1)?;
        }
        (items[k].cat == Cat::H).then_some(k)
    };

    // Classify the consonants from the end; the first one that takes
    // no below-base / post-base / pre-base-reordering form is the
    // base.
    let mut class = vec![0u8; n];
    let mut form_mask = vec![0u32; n];
    let base = if let Some(v) = vowel {
        for &c in consonants.iter().filter(|&&c| c > v) {
            let h = halant_before(items, c).map(|k| items[k].ch);
            let mut mask = 0;
            let mut pos = BELOW_C;
            if let Some(h) = h {
                if forms.joined_form(*b"pstf", h, items[c].ch) {
                    mask |= PSTF;
                    pos = POST_C;
                }
                if forms.joined_form(*b"blwf", h, items[c].ch) {
                    mask |= BLWF;
                    pos = BELOW_C;
                }
            }
            class[c] = pos;
            form_mask[c] = mask;
        }
        v
    } else if consonants.is_empty() {
        0
    } else {
        let first = if has_reph && consonants.len() > 1 {
            1
        } else {
            0
        };
        let mut base = consonants[first];
        let mut seen_below = false;
        for (rank, &c) in consonants.iter().enumerate().rev() {
            if rank == first {
                base = c;
                break;
            }
            let Some(h) = halant_before(items, c).map(|k| items[k].ch) else {
                base = c;
                break;
            };
            let ch = items[c].ch;
            let pre = script.ra.contains(&ch) && forms.joined_form(*b"pref", h, ch);
            let below = forms.joined_form(*b"blwf", h, ch);
            // Post-base forms have to follow below-base forms.
            let post = !seen_below && forms.joined_form(*b"pstf", h, ch);
            if !(pre || below || post) {
                base = c;
                break;
            }
            let mut mask = 0;
            if pre {
                mask |= PREF;
            }
            if below {
                mask |= BLWF;
                seen_below = true;
            }
            if post {
                mask |= PSTF;
            }
            class[c] = if below || pre { BELOW_C } else { POST_C };
            form_mask[c] = mask;
        }
        base
    };

    // Positions.
    let mut current = BASE;
    for i in 0..n {
        let cat = items[i].cat;
        if has_reph && i < 2 {
            items[i].pos = REPH;
            items[i].mask |= RPHF;
            continue;
        }
        if i < base {
            items[i].pos = PRE_C;
            continue;
        }
        if i == base {
            items[i].pos = BASE;
            current = BASE;
            continue;
        }
        match cat {
            Cat::C => {
                // Consonants keep their order among themselves.
                current = current.max(class[i].max(BELOW_C));
                items[i].pos = current;
                items[i].mask |= form_mask[i];
            }
            Cat::H => {
                // A halant belongs to the consonant it joins on, or —
                // at the end — to what it follows.
                let mut k = i + 1;
                if k < n && is_joiner(&items[k]) {
                    k += 1;
                }
                if k < n && items[k].cat == Cat::C {
                    items[i].pos = current.max(class[k].max(BELOW_C));
                    items[i].mask |= form_mask[k];
                } else {
                    items[i].pos = current;
                }
            }
            Cat::M => {
                current = sign_class(script, items[i].ch, cat);
                items[i].pos = current;
            }
            Cat::Sm | Cat::Vd => {
                current = sign_class(script, items[i].ch, cat);
                items[i].pos = current;
            }
            _ => items[i].pos = current,
        }
    }
    // A joiner between a halant and its consonant moves with them.
    for i in (base + 1)..n {
        if is_joiner(&items[i]) && i + 1 < n && items[i + 1].cat == Cat::C {
            items[i].pos = items[i + 1].pos;
        }
    }

    // Half forms: every pre-base consonant followed by a halant,
    // unless a ZERO WIDTH NON-JOINER asks for the explicit halant.
    for i in 0..base.min(n) {
        if items[i].pos != PRE_C || items[i].cat != Cat::C {
            continue;
        }
        let mut k = i + 1;
        if k < n && items[k].cat == Cat::N {
            k += 1;
        }
        if k < n && items[k].cat == Cat::H {
            let explicit = items.get(k + 1).is_some_and(|it| it.cat == Cat::Zwnj);
            if !explicit {
                for it in &mut items[i..=k] {
                    it.mask |= HALF;
                }
            }
        }
    }

    // The older model writes its below-base and post-base forms for
    // Consonant + Halant: give it that order.
    if forms.old_model {
        let mut i = base + 1;
        while i + 1 < n {
            if items[i].cat == Cat::H
                && items[i + 1].cat == Cat::C
                && items[i].pos == items[i + 1].pos
            {
                items.swap(i, i + 1);
                i += 2;
            } else {
                i += 1;
            }
        }
    }

    // Adjacent nukta and halant: the nukta goes first.
    for i in 1..n {
        if items[i].cat == Cat::N && items[i - 1].cat == Cat::H && items[i].pos == items[i - 1].pos
        {
            items.swap(i - 1, i);
        }
    }

    items.sort_by_key(|it| it.pos);
}

/// Move the glyph at `from` to sit just before index `to`.
fn move_glyph(glyphs: &mut [Glyph], from: usize, to: usize) {
    if from < to {
        glyphs[from..to].rotate_left(1);
    } else if to < from {
        glyphs[to..=from].rotate_right(1);
    }
}

/// "Final reordering" of the syllable occupying `glyphs[start..end]`.
fn final_reorder(script: &IndicScript, glyphs: &mut [Glyph], start: usize, end: usize) {
    let is_halant = |g: &Glyph| !g.ligated && category(g.ch) == Cat::H;
    let is_joiner = |g: &Glyph| matches!(g.ch, '\u{200c}' | '\u{200d}');
    let Some(mut base) = (start..end).find(|&i| glyphs[i].class == BASE) else {
        return;
    };

    // "After the last standalone halant glyph, after the initial matra
    // position and before the main consonant" (past a joiner that
    // follows the halant).
    let target_before_base = |glyphs: &[Glyph], after: usize, base: usize| -> Option<usize> {
        (after..base)
            .rev()
            .find(|&i| is_halant(&glyphs[i]))
            .map(|h| {
                let mut t = h + 1;
                while t < base && is_joiner(&glyphs[t]) {
                    t += 1;
                }
                t
            })
    };

    // Pre-base matras.
    let first_matra = (start..base).find(|&i| glyphs[i].class == BEFORE_HALF);
    if let Some(m0) = first_matra {
        let mut m1 = m0;
        while m1 < base && glyphs[m1].class == BEFORE_HALF {
            m1 += 1;
        }
        if let Some(t) = target_before_base(glyphs, m1, base) {
            glyphs[m0..t].rotate_left(m1 - m0);
        }
    }

    // A consonant that took its pre-base form moves in front of the
    // base (or to the matra's target).
    if let Some(p) = (base + 1..end).find(|&i| glyphs[i].class == PREF_FORMED) {
        let t = target_before_base(glyphs, start, base).unwrap_or(base);
        move_glyph(glyphs, p, t);
        base += 1;
    }

    // Reph.
    if glyphs[start].class == REPH && glyphs[start].ligated && start + 1 < end {
        let mut target = None;
        if script.reph != AFTER_POST {
            // After the first explicit halant between the first
            // consonant after the reph and the main consonant.
            target = (start + 1..base).find(|&i| is_halant(&glyphs[i])).map(|h| {
                let mut t = h + 1;
                while t < base && is_joiner(&glyphs[t]) {
                    t += 1;
                }
                t
            });
        }
        let target = target.unwrap_or_else(|| {
            // Before the first glyph after the base whose position is
            // past the reph's; else at the end of the syllable.
            (base + 1..end)
                .find(|&i| glyphs[i].class > script.reph && glyphs[i].class != PREF_FORMED)
                .unwrap_or(end)
        });
        move_glyph(glyphs, start, target);
    }
}

/// Shape a run of one of the covered scripts.
pub(crate) fn shape_indic(
    font: &Font<'_>,
    plan: &LayoutPlan,
    chars: &[RunChar],
    spec: &RunSpec<'_>,
    script: &IndicScript,
    tags: &PlainTags,
) -> Vec<RunGlyph> {
    let gsub = tags.gsub_features(font, plan, spec.opts.language);
    let own_tags = crate::script::ot_script_tags(spec.script);
    let old_model = own_tags.len() > 1 && tags.gsub == Some(own_tags[1]);
    let mut forms = Forms {
        font,
        plan,
        features: &gsub,
        old_model,
        keep: tags
            .gsub
            .filter(|_| spec.opts.language.is_none() && !font.gsub_has_feature_variations()),
    };

    // Split multi-part matras the script reorders by part.
    let mut split: Vec<RunChar> = Vec::with_capacity(chars.len() + 4);
    let mut parts: Vec<char> = Vec::new();
    for rc in chars {
        parts.clear();
        if category(rc.ch) == Cat::M
            && text::decompose(rc.ch, &mut parts)
            && parts
                .iter()
                .all(|&p| font.glyph_index(p).is_some_and(|g| g != 0))
        {
            split.extend(parts.iter().map(|&ch| RunChar {
                ch,
                cluster: rc.cluster,
            }));
        } else {
            split.push(*rc);
        }
    }
    let chars = &split[..];
    let cats: Vec<Cat> = chars.iter().map(|rc| category(rc.ch)).collect();

    // Syllables, reordered, as glyphs.
    let mut glyphs: Vec<Glyph> = Vec::with_capacity(chars.len());
    let mut items: Vec<Item> = Vec::new();
    let mut serial: u16 = 0;
    let mut start = 0usize;
    let mut word_start = true;
    while start < chars.len() {
        let (end, well_formed) = syllable_end(&cats, start);
        serial = serial.wrapping_add(1).max(1);
        items.clear();
        items.extend((start..end).map(|i| Item {
            ch: chars[i].ch,
            cluster: chars[i].cluster,
            cat: cats[i],
            pos: BASE,
            mask: GLOBAL,
        }));
        if well_formed {
            reorder_syllable(script, &mut forms, &mut items);
        }
        let cluster = items.iter().map(|it| it.cluster).min().unwrap_or(0);
        let first = glyphs.len();
        for it in &items {
            let mut g = Glyph::new(font.glyph_index(it.ch).unwrap_or(0), it.ch, cluster);
            g.ignorable = text::is_default_ignorable(it.ch);
            g.mask = it.mask;
            g.class = if well_formed { it.pos } else { 0 };
            g.group = serial;
            glyphs.push(g);
        }
        if well_formed && word_start {
            glyphs[first].mask |= INIT;
        }
        word_start = !well_formed && !chars[start].ch.is_alphanumeric();
        start = end;
    }

    let mut buffer = Buffer::new(glyphs);
    buffer.bounded = true;
    // "The presence of ZWJ will prevent GSUB substitution lookups from
    // matching the input glyph sequence."
    buffer.joiners_block = true;

    let active = active_features(spec, &[]);
    let on = |tag: &[u8; 4]| active.contains(tag);
    let value = |tag: [u8; 4]| spec.opts.feature(tag).unwrap_or(1);

    // Localized forms, then the basic shaping forms one at a time.
    let basic: [([u8; 4], u32); 12] = [
        (*b"locl", GLOBAL),
        (*b"nukt", GLOBAL),
        (*b"akhn", GLOBAL),
        (*b"rphf", RPHF),
        (*b"rkrf", GLOBAL),
        (*b"pref", PREF),
        (*b"blwf", BLWF),
        (*b"abvf", ABVF),
        (*b"half", HALF),
        (*b"pstf", PSTF),
        (*b"vatu", GLOBAL),
        (*b"cjct", GLOBAL),
    ];
    let pre: Vec<([u8; 4], u32, u32)> = [*b" RQD", *b"rvrn", *b"ccmp"]
        .into_iter()
        .filter(|t| t == b" RQD" || on(t))
        .map(|t| (t, GLOBAL, 1))
        .collect();
    let lang = spec.opts.language;
    let lookups = tags.stage(font, plan, lang, &pre, false);
    buffer.substitute(font, plan, &lookups);
    for (tag, mask) in basic {
        if tag == *b"locl" && !on(&tag) {
            continue;
        }
        let lookups = tags.stage(font, plan, lang, &[(tag, mask, 1)], false);
        if lookups.is_empty() {
            continue;
        }
        buffer.substitute(font, plan, &lookups);
        if &tag == b"pref" {
            for g in &mut buffer.glyphs {
                if g.mask & PREF != 0 && g.ligated && g.class != BASE {
                    g.class = PREF_FORMED;
                }
            }
        }
    }

    // Final reordering, syllable by syllable.
    let mut s = 0usize;
    while s < buffer.glyphs.len() {
        let group = buffer.glyphs[s].group;
        let mut e = s + 1;
        while e < buffer.glyphs.len() && buffer.glyphs[e].group == group {
            e += 1;
        }
        if buffer.glyphs[s..e].iter().any(|g| g.class != 0) {
            final_reorder(script, &mut buffer.glyphs, s, e);
        }
        s = e;
    }

    buffer.invalidate();

    // Presentation forms, together.
    let basic_tags: Vec<[u8; 4]> = basic.iter().map(|b| b.0).collect();
    let positioning: [[u8; 4]; 7] = [
        *b"kern", *b"dist", *b"abvm", *b"blwm", *b"mark", *b"mkmk", *b"curs",
    ];
    let mut stage: Vec<([u8; 4], u32, u32)> = vec![(*b"init", INIT, 1)];
    for t in [*b"pres", *b"abvs", *b"blws", *b"psts", *b"haln"] {
        stage.push((t, GLOBAL, 1));
    }
    for &t in &active {
        if !basic_tags.contains(&t)
            && !positioning.contains(&t)
            && !matches!(&t, b"rvrn" | b"ccmp" | b"ltra" | b"ltrm" | b"rtla")
        {
            stage.push((t, GLOBAL, value(t)));
        }
    }
    let lookups = tags.stage(font, plan, lang, &stage, false);
    buffer.substitute(font, plan, &lookups);

    // Positioning runs over the whole run.
    buffer.bounded = false;
    buffer.invalidate();
    let stage: Vec<([u8; 4], u32, u32)> = active
        .iter()
        .filter(|t| positioning.contains(t))
        .map(|&t| (t, GLOBAL, value(t)))
        .chain(std::iter::once((*b" RQD", GLOBAL, 1)))
        .collect();
    let lookups = tags.stage(font, plan, lang, &stage, true);
    buffer.position(font, plan, &lookups, false, false, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cats(text: &str) -> Vec<Cat> {
        text.chars().map(category).collect()
    }

    fn syllables(text: &str) -> Vec<usize> {
        let c = cats(text);
        let mut ends = Vec::new();
        let mut i = 0;
        while i < c.len() {
            let (e, _) = syllable_end(&c, i);
            ends.push(e);
            i = e;
        }
        ends
    }

    #[test]
    fn syllable_patterns() {
        // KA + HALANT + SSA + I-matra + anusvara: one consonant syllable.
        assert_eq!(syllables("\u{915}\u{94d}\u{937}\u{93f}\u{902}"), [5]);
        // KA, then KA + AA: two syllables.
        assert_eq!(syllables("\u{915}\u{915}\u{93e}"), [1, 3]);
        // A final halant ends its syllable; the next consonant starts
        // a new one only when no halant joins them.
        assert_eq!(syllables("\u{915}\u{94d} \u{915}"), [2, 3, 4]);
        // Halant + ZWNJ + consonant stays one syllable (explicit
        // halant form).
        assert_eq!(syllables("\u{915}\u{94d}\u{200c}\u{937}"), [4]);
        // Independent vowel with a sign; RA + HALANT before a vowel.
        assert_eq!(syllables("\u{905}\u{902}"), [2]);
        assert_eq!(syllables("\u{930}\u{94d}\u{907}"), [3]);
        // A lone matra is not a syllable of its own pattern.
        let c = cats("\u{93f}");
        assert_eq!(syllable_end(&c, 0), (1, false));
        // Latin text is never grouped.
        assert_eq!(syllables("ab"), [1, 2]);
    }

    #[test]
    fn categories_come_from_the_character_database() {
        assert_eq!(category('\u{915}'), Cat::C);
        assert_eq!(category('\u{905}'), Cat::V);
        assert_eq!(category('\u{93c}'), Cat::N);
        assert_eq!(category('\u{94d}'), Cat::H);
        assert_eq!(category('\u{93f}'), Cat::M);
        assert_eq!(category('\u{902}'), Cat::Sm);
        assert_eq!(category('\u{952}'), Cat::Vd);
        assert_eq!(category('\u{a0}'), Cat::Placeholder);
        assert_eq!(category('x'), Cat::Other);
    }

    #[test]
    fn script_tables() {
        let deva = indic_script(Script::Devanagari).unwrap();
        assert_eq!(sign_class(deva, '\u{93f}', Cat::M), BEFORE_HALF);
        assert_eq!(sign_class(deva, '\u{940}', Cat::M), AFTER_SUB);
        assert_eq!(sign_class(deva, '\u{902}', Cat::Sm), SMVD);
        let beng = indic_script(Script::Bengali).unwrap();
        assert_eq!(sign_class(beng, '\u{9c7}', Cat::M), BEFORE_HALF);
        assert_eq!(sign_class(beng, '\u{9be}', Cat::M), AFTER_POST);
        assert_eq!(beng.reph, AFTER_SUB);
        let taml = indic_script(Script::Tamil).unwrap();
        assert_eq!(sign_class(taml, '\u{bc6}', Cat::M), BEFORE_MAIN);
        assert!(indic_script(Script::Latin).is_none());
    }

    #[test]
    fn glyph_moves() {
        let g = |n: u16| Glyph::new(n, '\0', 0);
        let ids = |v: &[Glyph]| v.iter().map(|g| g.gid).collect::<Vec<_>>();
        let mut v = vec![g(1), g(2), g(3), g(4)];
        move_glyph(&mut v, 0, 3);
        assert_eq!(ids(&v), [2, 3, 1, 4]);
        move_glyph(&mut v, 3, 0);
        assert_eq!(ids(&v), [4, 2, 3, 1]);
        move_glyph(&mut v, 1, 1);
        assert_eq!(ids(&v), [4, 2, 3, 1]);
    }
}
