//! Cursive-joining scripts (Arabic, Syriac, N'Ko, Mongolian, …) on the
//! staged engine.
//!
//! The procedure is the one the Arabic script-development
//! specification lays out ("Analyzing the characters", "Shaping with
//! OTLS", "Positioning glyphs with OTLS"):
//!
//! 1. every character is given a contextual form from its
//!    `Joining_Type` and that of its neighbours — transparent
//!    characters (marks, most format characters) are looked through;
//! 2. glyphs take their nominal form from `cmap`;
//! 3. substitution features run in the fixed order `ccmp` (with the
//!    language forms), `isol`, `fina`, `medi`, `init`, `rlig`, `calt`,
//!    then the typographic features — each form feature only on the
//!    glyphs that were assigned that form;
//! 4. positioning features (`curs`, `kern`, `mark`, `mkmk`) follow.
//!
//! A font without form features under the script falls back to the
//! Arabic Presentation Forms code points for the forms it has.

use crate::shaping::arabic::JoiningForm;
use crate::shaping::arabic_pf::presentation_form;
use crate::shaping::engine::{PlainTags, RunChar, RunGlyph, RunSpec, active_features, base_glyphs};
use crate::shaping::plan::LayoutPlan;
use crate::shaping::staged::{Buffer, GLOBAL, plan_stage};
use intl::unicode::JoiningType;
use oxideav_ttf::Font;

const ISOL: u32 = 1 << 1;
const FINA: u32 = 1 << 2;
const MEDI: u32 = 1 << 3;
const INIT: u32 = 1 << 4;

/// `true` when `c` takes part in cursive joining (dual-, right- or
/// left-joining).
pub(crate) fn is_joining(c: char) -> bool {
    (c as u32) >= 0x600
        && matches!(
            intl::unicode::joining_type(c),
            JoiningType::DualJoining | JoiningType::RightJoining | JoiningType::LeftJoining
        )
}

/// The contextual form of every character: `None` for characters that
/// have no forms (non-joining, join-causing, transparent).
///
/// A character joins towards the *preceding* character when it is
/// dual- or right-joining and the nearest non-transparent character
/// before it is dual-joining, left-joining or join-causing; towards
/// the *following* character when it is dual- or left-joining and the
/// nearest non-transparent character after it is dual-joining,
/// right-joining or join-causing. Both: medial; only the preceding
/// side: final; only the following side: initial; neither: isolated.
pub(crate) fn contextual_forms(chars: &[RunChar]) -> Vec<Option<JoiningForm>> {
    use JoiningType as T;
    let types: Vec<JoiningType> = chars
        .iter()
        .map(|rc| {
            if (rc.ch as u32) < 0x300 {
                T::NonJoining
            } else {
                intl::unicode::joining_type(rc.ch)
            }
        })
        .collect();
    let mut forms = vec![None; chars.len()];
    let mut prev: Option<JoiningType> = None;
    for i in 0..chars.len() {
        let t = types[i];
        if t == T::Transparent {
            continue;
        }
        if matches!(t, T::DualJoining | T::RightJoining | T::LeftJoining) {
            let next = types[i + 1..]
                .iter()
                .copied()
                .find(|&n| n != T::Transparent);
            let joins_prev = matches!(t, T::DualJoining | T::RightJoining)
                && matches!(prev, Some(T::DualJoining | T::LeftJoining | T::JoinCausing));
            let joins_next = matches!(t, T::DualJoining | T::LeftJoining)
                && matches!(
                    next,
                    Some(T::DualJoining | T::RightJoining | T::JoinCausing)
                );
            forms[i] = Some(match (joins_prev, joins_next) {
                (true, true) => JoiningForm::Medi,
                (true, false) => JoiningForm::Fina,
                (false, true) => JoiningForm::Init,
                (false, false) => JoiningForm::Isol,
            });
        }
        prev = Some(t);
    }
    forms
}

/// Shape a run of a cursive-joining script.
pub(crate) fn shape_joining(
    font: &Font<'_>,
    plan: &LayoutPlan,
    chars: &[RunChar],
    spec: &RunSpec<'_>,
    tags: &PlainTags,
) -> Vec<RunGlyph> {
    let forms = contextual_forms(chars);
    let gsub = tags.gsub_features(font, plan, spec.opts.language);
    let has_form_features = gsub
        .iter()
        .any(|f| matches!(&f.tag, b"init" | b"medi" | b"fina" | b"isol"));

    // Without form features in the font: the presentation-form code
    // points stand in for the forms the font draws.
    let mut chars: std::borrow::Cow<'_, [RunChar]> = chars.into();
    if !has_form_features {
        for (i, form) in forms.iter().enumerate() {
            if let Some(form) = *form
                && let Some(pf) = presentation_form(chars[i].ch, form)
                && font.glyph_index(pf).is_some_and(|g| g != 0)
            {
                chars.to_mut()[i].ch = pf;
            }
        }
    }

    let (mut glyphs, sources) = base_glyphs(font, &chars);
    for (g, &ci) in glyphs.iter_mut().zip(&sources) {
        g.mask = GLOBAL
            | match forms[ci] {
                Some(JoiningForm::Isol) => ISOL,
                Some(JoiningForm::Fina) => FINA,
                Some(JoiningForm::Medi) => MEDI,
                Some(JoiningForm::Init) => INIT,
                None => 0,
            };
    }
    let mut buffer = Buffer::new(glyphs);

    // The caller's switches: a default feature can be turned off, any
    // other feature turned on (it joins the last stage).
    let active = active_features(spec, &[*b"mset", *b"cswh"]);
    let on = |tag: &[u8; 4]| active.contains(tag);
    let value = |tag: [u8; 4]| spec.opts.feature(tag).unwrap_or(1);
    let stage_of = |tags: &[[u8; 4]], mask: u32| -> Vec<([u8; 4], u32, u32)> {
        tags.iter()
            .filter(|t| on(t))
            .map(|&t| (t, mask, value(t)))
            .collect()
    };
    let staged_tags: [&[[u8; 4]]; 8] = [
        &[
            *b" RQD", *b"rvrn", *b"ccmp", *b"locl", *b"rtla", *b"ltra", *b"ltrm",
        ],
        &[*b"isol"],
        &[*b"fina"],
        &[*b"medi"],
        &[*b"init"],
        &[*b"rlig"],
        &[*b"rclt", *b"calt"],
        &[],
    ];
    let masks = [GLOBAL, ISOL, FINA, MEDI, INIT, GLOBAL, GLOBAL, GLOBAL];
    let positioning: [[u8; 4]; 7] = [
        *b"curs", *b"kern", *b"mark", *b"mkmk", *b"dist", *b"abvm", *b"blwm",
    ];
    for (k, (tags_k, mask)) in staged_tags.iter().zip(masks).enumerate() {
        let mut stage: Vec<([u8; 4], u32, u32)> = if (1..=4).contains(&k) {
            // The form features are part of the analysis, not options.
            tags_k.iter().map(|&t| (t, mask, 1)).collect()
        } else if k == 0 {
            let mut s = stage_of(tags_k, mask);
            s.insert(0, (*b" RQD", GLOBAL, 1));
            s
        } else {
            stage_of(tags_k, mask)
        };
        if k + 1 == staged_tags.len() {
            // Everything else that is active and substitutes.
            for &t in &active {
                let earlier = staged_tags.iter().any(|s| s.contains(&t));
                if !earlier && !positioning.contains(&t) {
                    stage.push((t, GLOBAL, value(t)));
                }
            }
        }
        if stage.is_empty() {
            continue;
        }
        let lookups = plan_stage(font, plan, &gsub, &stage, false);
        buffer.substitute(font, &lookups);
    }

    let gpos = tags.gpos_features(font, plan, spec.opts.language);
    let stage: Vec<([u8; 4], u32, u32)> = active
        .iter()
        .map(|&t| (t, GLOBAL, value(t)))
        .chain(std::iter::once((*b" RQD", GLOBAL, 1)))
        .collect();
    let lookups = plan_stage(font, plan, &gpos, &stage, true);
    let kern_on = on(b"kern") && !spec.opts.vertical;
    let gpos_kerns = gpos.iter().any(|f| &f.tag == b"kern");
    let legacy_kern = kern_on && !gpos_kerns && font.kern_header_variant().is_some();
    buffer.position(font, &lookups, legacy_kern, true, spec.rtl)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forms(text: &str) -> Vec<Option<JoiningForm>> {
        let chars: Vec<RunChar> = text.chars().map(|ch| RunChar { ch, cluster: 0 }).collect();
        contextual_forms(&chars)
    }

    #[test]
    fn forms_follow_joining_types() {
        use JoiningForm::*;
        // beh beh beh: initial, medial, final.
        assert_eq!(
            forms("\u{628}\u{628}\u{628}"),
            [Some(Init), Some(Medi), Some(Fina)]
        );
        // alef is right-joining: it ends a chain and never starts one.
        assert_eq!(
            forms("\u{628}\u{627}\u{628}"),
            [Some(Init), Some(Fina), Some(Isol)]
        );
        // A mark between two letters is looked through.
        assert_eq!(
            forms("\u{628}\u{64e}\u{628}"),
            [Some(Init), None, Some(Fina)]
        );
        // Tatweel and ZERO WIDTH JOINER cause joining; ZERO WIDTH
        // NON-JOINER and a space prevent it.
        assert_eq!(forms("\u{628}\u{640}"), [Some(Init), None]);
        assert_eq!(forms("\u{200d}\u{628}"), [None, Some(Fina)]);
        assert_eq!(
            forms("\u{628}\u{200c}\u{628}"),
            [Some(Isol), None, Some(Isol)]
        );
        assert_eq!(forms("\u{628} \u{628}"), [Some(Isol), None, Some(Isol)]);
        assert_eq!(forms("ab"), [None, None]);
    }
}
