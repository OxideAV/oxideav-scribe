//! Map `(base Arabic codepoint, JoiningForm)` → Arabic Presentation
//! Forms-B codepoint (U+FE70..U+FEFF).
//!
//! Most Arabic-capable fonts (DejaVuSans, Noto Sans Arabic, Amiri,
//! etc.) ship glyphs for the Presentation Forms-B block in addition to
//! (or instead of) GSUB feature lookups. This lets a clean-room shaper
//! emit visually-correct contextual forms even without parsing
//! feature-tagged GSUB lookups: pick the form via the joining state
//! machine in [`super::arabic`], then translate `(base, form)` into the
//! corresponding presentation-form codepoint and re-cmap.
//!
//! ## Coverage
//!
//! - Hamza variants (FE80..FE8C).
//! - Alef family (FE8D..FE8E).
//! - Core Arabic alphabet BEH..YEH (FE8F..FEF4) — every dual-joining
//!   letter has all four forms; right-joining letters have isol + fina
//!   only.
//!
//! Round-7 ignores the Presentation Forms-A block (FB50..FDFF) which
//! holds extended Persian / Urdu letters and the LAM-ALEF ligatures —
//! those are a separate substitution pass we'll add in a later round.
//! Falling through to `None` means the shaper keeps the original base
//! codepoint, which still renders (just in isolated form).
//!
//! ## Source
//!
//! Mapping derived from the Unicode 15.1 character database
//! (`UnicodeData.txt` decomposition mappings for FE70..FEFF — every
//! presentation-form has a `<initial>` / `<medial>` / `<final>` /
//! `<isolated>` decomposition tag pointing back to the base
//! codepoint). No HarfBuzz / FreeType / ICU source consulted.

use super::arabic::JoiningForm;

/// Translate `(base, form)` to its Arabic Presentation Forms-B
/// codepoint, or `None` if the base codepoint isn't in the mapping
/// table for that form.
///
/// Returning `None` is the safe default — the shaper keeps the
/// original codepoint, which still renders (just always in isolated
/// shape).
pub fn presentation_form(base: char, form: JoiningForm) -> Option<char> {
    let cp = base as u32;
    let (isol, fina, init, medi) = forms_for(cp)?;
    // For each requested form, return the specific PF-B codepoint when
    // the table has it. If the form is missing for this base letter
    // (e.g. R-class letters have no init/medi), return `None` so the
    // caller can fall back to the original base codepoint.
    let chosen = match form {
        JoiningForm::Isol => Some(isol),
        JoiningForm::Fina => fina,
        JoiningForm::Init => init,
        JoiningForm::Medi => medi,
    }?;
    char::from_u32(chosen)
}

/// Per-base PF-B mapping: the 4 contextual forms, with the isolated
/// form always present and the others optional. R-class letters carry
/// `None` for `init` / `medi` (they have no glyph for those positions);
/// HAMZA carries `None` for everything except `isol`.
type PfFormSet = (u32, Option<u32>, Option<u32>, Option<u32>);

/// Per-base table: `(isol, fina, init, medi)` codepoints (None means
/// "this letter doesn't have that form in the Presentation Forms-B
/// block").
fn forms_for(cp: u32) -> Option<PfFormSet> {
    match cp {
        // HAMZA — isolated only.
        0x0621 => Some((0xFE80, None, None, None)),
        // Hamza-on-base R-class letters: 2 forms.
        0x0622 => Some((0xFE81, Some(0xFE82), None, None)),
        0x0623 => Some((0xFE83, Some(0xFE84), None, None)),
        0x0624 => Some((0xFE85, Some(0xFE86), None, None)),
        0x0625 => Some((0xFE87, Some(0xFE88), None, None)),
        // YEH WITH HAMZA ABOVE — D-class, 4 forms.
        0x0626 => Some((0xFE89, Some(0xFE8A), Some(0xFE8B), Some(0xFE8C))),
        // ALEF — R-class, 2 forms.
        0x0627 => Some((0xFE8D, Some(0xFE8E), None, None)),
        // BEH — D-class, 4 forms.
        0x0628 => Some((0xFE8F, Some(0xFE90), Some(0xFE91), Some(0xFE92))),
        // TEH MARBUTA — R-class, 2 forms.
        0x0629 => Some((0xFE93, Some(0xFE94), None, None)),
        // TEH — D-class, 4 forms.
        0x062A => Some((0xFE95, Some(0xFE96), Some(0xFE97), Some(0xFE98))),
        // THEH — D-class, 4 forms.
        0x062B => Some((0xFE99, Some(0xFE9A), Some(0xFE9B), Some(0xFE9C))),
        // JEEM — D-class.
        0x062C => Some((0xFE9D, Some(0xFE9E), Some(0xFE9F), Some(0xFEA0))),
        // HAH — D-class.
        0x062D => Some((0xFEA1, Some(0xFEA2), Some(0xFEA3), Some(0xFEA4))),
        // KHAH — D-class.
        0x062E => Some((0xFEA5, Some(0xFEA6), Some(0xFEA7), Some(0xFEA8))),
        // DAL — R-class.
        0x062F => Some((0xFEA9, Some(0xFEAA), None, None)),
        // THAL — R-class.
        0x0630 => Some((0xFEAB, Some(0xFEAC), None, None)),
        // REH — R-class.
        0x0631 => Some((0xFEAD, Some(0xFEAE), None, None)),
        // ZAIN — R-class.
        0x0632 => Some((0xFEAF, Some(0xFEB0), None, None)),
        // SEEN — D-class.
        0x0633 => Some((0xFEB1, Some(0xFEB2), Some(0xFEB3), Some(0xFEB4))),
        // SHEEN — D-class.
        0x0634 => Some((0xFEB5, Some(0xFEB6), Some(0xFEB7), Some(0xFEB8))),
        // SAD — D-class.
        0x0635 => Some((0xFEB9, Some(0xFEBA), Some(0xFEBB), Some(0xFEBC))),
        // DAD — D-class.
        0x0636 => Some((0xFEBD, Some(0xFEBE), Some(0xFEBF), Some(0xFEC0))),
        // TAH — D-class.
        0x0637 => Some((0xFEC1, Some(0xFEC2), Some(0xFEC3), Some(0xFEC4))),
        // ZAH — D-class.
        0x0638 => Some((0xFEC5, Some(0xFEC6), Some(0xFEC7), Some(0xFEC8))),
        // AIN — D-class.
        0x0639 => Some((0xFEC9, Some(0xFECA), Some(0xFECB), Some(0xFECC))),
        // GHAIN — D-class.
        0x063A => Some((0xFECD, Some(0xFECE), Some(0xFECF), Some(0xFED0))),
        // FEH — D-class.
        0x0641 => Some((0xFED1, Some(0xFED2), Some(0xFED3), Some(0xFED4))),
        // QAF — D-class.
        0x0642 => Some((0xFED5, Some(0xFED6), Some(0xFED7), Some(0xFED8))),
        // KAF — D-class.
        0x0643 => Some((0xFED9, Some(0xFEDA), Some(0xFEDB), Some(0xFEDC))),
        // LAM — D-class.
        0x0644 => Some((0xFEDD, Some(0xFEDE), Some(0xFEDF), Some(0xFEE0))),
        // MEEM — D-class.
        0x0645 => Some((0xFEE1, Some(0xFEE2), Some(0xFEE3), Some(0xFEE4))),
        // NOON — D-class.
        0x0646 => Some((0xFEE5, Some(0xFEE6), Some(0xFEE7), Some(0xFEE8))),
        // HEH — D-class.
        0x0647 => Some((0xFEE9, Some(0xFEEA), Some(0xFEEB), Some(0xFEEC))),
        // WAW — R-class.
        0x0648 => Some((0xFEED, Some(0xFEEE), None, None)),
        // ALEF MAKSURA — historically R-class in the Presentation
        // Forms-B block (only isol + fina). Modern fonts that render
        // it as D rely on GSUB, which we don't have yet — this is the
        // safe black-box mapping.
        0x0649 => Some((0xFEEF, Some(0xFEF0), None, None)),
        // YEH — D-class.
        0x064A => Some((0xFEF1, Some(0xFEF2), Some(0xFEF3), Some(0xFEF4))),
        // Tatweel maps to itself (no separate presentation form).
        0x0640 => Some((0x0640, None, None, None)),
        _ => None,
    }
}

#[cfg(test)]
#[allow(non_snake_case)] // Tests reference PF-B codepoints (FE8F, etc.)
                         // by their canonical hex spelling.
mod tests {
    use super::*;

    #[test]
    fn beh_isolated_maps_to_FE8F() {
        assert_eq!(
            presentation_form('\u{0628}', JoiningForm::Isol),
            Some('\u{FE8F}')
        );
    }

    #[test]
    fn beh_initial_maps_to_FE91() {
        assert_eq!(
            presentation_form('\u{0628}', JoiningForm::Init),
            Some('\u{FE91}')
        );
    }

    #[test]
    fn beh_medial_maps_to_FE92() {
        assert_eq!(
            presentation_form('\u{0628}', JoiningForm::Medi),
            Some('\u{FE92}')
        );
    }

    #[test]
    fn beh_final_maps_to_FE90() {
        assert_eq!(
            presentation_form('\u{0628}', JoiningForm::Fina),
            Some('\u{FE90}')
        );
    }

    #[test]
    fn alef_initial_falls_back_to_isol_form() {
        // ALEF is R-class — no init form. We expect None: the shaper
        // should keep the base codepoint (or pick isol).
        // In practice the joining state machine never assigns Init to
        // an R-class letter, but the table must be defensive.
        assert_eq!(presentation_form('\u{0627}', JoiningForm::Init), None);
    }

    #[test]
    fn alef_final_maps_to_FE8E() {
        assert_eq!(
            presentation_form('\u{0627}', JoiningForm::Fina),
            Some('\u{FE8E}')
        );
    }

    #[test]
    fn lam_all_four_forms() {
        assert_eq!(
            presentation_form('\u{0644}', JoiningForm::Isol),
            Some('\u{FEDD}')
        );
        assert_eq!(
            presentation_form('\u{0644}', JoiningForm::Fina),
            Some('\u{FEDE}')
        );
        assert_eq!(
            presentation_form('\u{0644}', JoiningForm::Init),
            Some('\u{FEDF}')
        );
        assert_eq!(
            presentation_form('\u{0644}', JoiningForm::Medi),
            Some('\u{FEE0}')
        );
    }

    #[test]
    fn unmapped_codepoint_returns_none() {
        assert_eq!(presentation_form('A', JoiningForm::Isol), None);
    }
}
