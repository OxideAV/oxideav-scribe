//! Unicode `Script` → OpenType script-tag resolution and script-run
//! segmentation.
//!
//! An OpenType `GSUB` / `GPOS` table organises its lookups under a
//! `ScriptList` keyed by **4-byte script tags** (`b"latn"`, `b"arab"`,
//! `b"deva"`, …). To shape a run of text the engine must map the run's
//! Unicode script (the `Script` property, UAX #24) to the matching
//! OpenType script tag so it can select the right `ScriptList` entry —
//! and, upstream of that, it must split a mixed-script string into
//! maximal same-script runs so each run is shaped under its own tag.
//!
//! This module provides both halves:
//!
//! * [`ot_script_tag`] / [`ot_script_tags`] — the Unicode-script →
//!   OpenType-tag lookup. The tag values are transcribed from the
//!   **OpenType Layout — Script Tags** registry staged under
//!   `docs/text/opentype/registries/script-tags.html` (© Microsoft
//!   Corporation, OpenType specification, licensed under CC-BY-4.0).
//!   Scripts that the registry assigns both a legacy and a "v.2" shaping
//!   tag (the Indic scripts) return the pair, modern-tag-first, from
//!   [`ot_script_tags`].
//! * [`ScriptRun`] / [`script_runs`] — itemise a string into maximal
//!   same-script runs, resolving the `Common` and `Inherited` pseudo-
//!   scripts onto a neighbouring real script so a run like
//!   `"abc, def"` (where the comma and space are `Common`) stays one
//!   Latin run rather than fragmenting on the punctuation.
//!
//! The Unicode `Script` property itself is supplied by the `intl`
//! crate's compiled UCD tables ([`intl::unicode::script::script`]); this
//! module never re-derives it.

use intl::unicode::script::{script, Script};

/// Resolve a Unicode [`Script`] to its primary OpenType script tag.
///
/// Returns the modern shaping tag for scripts the OpenType registry
/// gives a "v.2" form (e.g. Devanagari → `b"dev2"`, not the legacy
/// `b"deva"`); use [`ot_script_tags`] when you need both tags so you can
/// fall back to the legacy form for older fonts that only register it.
///
/// `Script::Common`, `Script::Inherited`, and `Script::Unknown` resolve
/// to the OpenType **Default** tag `b"DFLT"` — the `ScriptList` entry a
/// font publishes for "text with no script-specific behaviour". A run
/// that is genuinely script-less (digits, punctuation) is shaped under
/// `DFLT`.
///
/// Provenance: OpenType Layout *Script Tags* registry,
/// `docs/text/opentype/registries/script-tags.html`
/// (© Microsoft Corporation, CC-BY-4.0).
#[must_use]
pub fn ot_script_tag(s: Script) -> [u8; 4] {
    ot_script_tags(s)[0]
}

/// Resolve a Unicode [`Script`] to its OpenType script tag(s), modern
/// tag first.
///
/// Most scripts have a single registered tag, so the returned slice has
/// length 1. The Indic scripts that the registry lists with both a
/// legacy tag and a "v.2" tag return a two-element slice `[modern,
/// legacy]` — a shaper looks up the modern tag in the font's
/// `ScriptList` first and falls back to the legacy tag if the font does
/// not register the v.2 form. The pairs (registry display name →
/// `[modern, legacy]`):
///
/// | Script | modern | legacy |
/// |--------|--------|--------|
/// | Bengali / Bangla | `bng2` | `beng` |
/// | Devanagari | `dev2` | `deva` |
/// | Gujarati | `gjr2` | `gujr` |
/// | Gurmukhi | `gur2` | `guru` |
/// | Kannada | `knd2` | `knda` |
/// | Malayalam | `mlm2` | `mlym` |
/// | Oriya / Odia | `ory2` | `orya` |
/// | Tamil | `tml2` | `taml` |
/// | Telugu | `tel2` | `telu` |
/// | Myanmar | `mym2` | `mymr` |
///
/// Provenance: OpenType Layout *Script Tags* registry,
/// `docs/text/opentype/registries/script-tags.html`
/// (© Microsoft Corporation, CC-BY-4.0).
#[must_use]
pub fn ot_script_tags(s: Script) -> &'static [[u8; 4]] {
    // Scripts with a "v.2" shaping tag: modern first, legacy second.
    // `const { .. }` forces the array into a static so the returned
    // reference is `'static` (a plain `&[..]` in a match arm is only
    // block-scoped and would not outlive the function).
    macro_rules! pair {
        ($m:literal, $l:literal) => {
            const { &[*$m, *$l] }
        };
    }
    macro_rules! one {
        ($t:literal) => {
            const { &[*$t] }
        };
    }
    match s {
        // Indic dual-tag scripts (legacy + v.2 shaping engine).
        Script::Bengali => pair!(b"bng2", b"beng"),
        Script::Devanagari => pair!(b"dev2", b"deva"),
        Script::Gujarati => pair!(b"gjr2", b"gujr"),
        Script::Gurmukhi => pair!(b"gur2", b"guru"),
        Script::Kannada => pair!(b"knd2", b"knda"),
        Script::Malayalam => pair!(b"mlm2", b"mlym"),
        Script::Oriya => pair!(b"ory2", b"orya"),
        Script::Tamil => pair!(b"tml2", b"taml"),
        Script::Telugu => pair!(b"tel2", b"telu"),
        Script::Myanmar => pair!(b"mym2", b"mymr"),

        // Single-tag scripts (the common shaping repertoire first).
        Script::Latin => one!(b"latn"),
        Script::Cyrillic => one!(b"cyrl"),
        Script::Greek => one!(b"grek"),
        Script::Arabic => one!(b"arab"),
        Script::Hebrew => one!(b"hebr"),
        Script::Han => one!(b"hani"),
        Script::Hiragana | Script::Katakana => one!(b"kana"),
        Script::Hangul => one!(b"hang"),
        Script::Bopomofo => one!(b"bopo"),
        Script::Thai => one!(b"thai"),
        Script::Lao => one!(b"lao "),
        Script::Khmer => one!(b"khmr"),
        Script::Tibetan => one!(b"tibt"),
        Script::Sinhala => one!(b"sinh"),
        Script::Syriac => one!(b"syrc"),
        Script::Thaana => one!(b"thaa"),
        Script::Nko => one!(b"nko "),
        Script::Ethiopic => one!(b"ethi"),
        Script::Armenian => one!(b"armn"),
        Script::Georgian => one!(b"geor"),
        Script::Mongolian => one!(b"mong"),

        // Remaining registered scripts the shaper might encounter.
        Script::Adlam => one!(b"adlm"),
        Script::Ahom => one!(b"ahom"),
        Script::AnatolianHieroglyphs => one!(b"hluw"),
        Script::Avestan => one!(b"avst"),
        Script::Balinese => one!(b"bali"),
        Script::Bamum => one!(b"bamu"),
        Script::BassaVah => one!(b"bass"),
        Script::Batak => one!(b"batk"),
        Script::Bhaiksuki => one!(b"bhks"),
        Script::Brahmi => one!(b"brah"),
        Script::Braille => one!(b"brai"),
        Script::Buginese => one!(b"bugi"),
        Script::Buhid => one!(b"buhd"),
        Script::CanadianAboriginal => one!(b"cans"),
        Script::Carian => one!(b"cari"),
        Script::CaucasianAlbanian => one!(b"aghb"),
        Script::Chakma => one!(b"cakm"),
        Script::Cham => one!(b"cham"),
        Script::Cherokee => one!(b"cher"),
        Script::Chorasmian => one!(b"chrs"),
        Script::Coptic => one!(b"copt"),
        Script::Cuneiform => one!(b"xsux"),
        Script::Cypriot => one!(b"cprt"),
        Script::CyproMinoan => one!(b"cpmn"),
        Script::Deseret => one!(b"dsrt"),
        Script::DivesAkuru => one!(b"diak"),
        Script::Dogra => one!(b"dogr"),
        Script::Duployan => one!(b"dupl"),
        Script::EgyptianHieroglyphs => one!(b"egyp"),
        Script::Elbasan => one!(b"elba"),
        Script::Elymaic => one!(b"elym"),
        Script::Garay => one!(b"gara"),
        Script::Glagolitic => one!(b"glag"),
        Script::Gothic => one!(b"goth"),
        Script::Grantha => one!(b"gran"),
        Script::GunjalaGondi => one!(b"gong"),
        Script::GurungKhema => one!(b"gukh"),
        Script::HanifiRohingya => one!(b"rohg"),
        Script::Hanunoo => one!(b"hano"),
        Script::Hatran => one!(b"hatr"),
        Script::ImperialAramaic => one!(b"armi"),
        Script::InscriptionalPahlavi => one!(b"phli"),
        Script::InscriptionalParthian => one!(b"prti"),
        Script::Javanese => one!(b"java"),
        Script::Kaithi => one!(b"kthi"),
        Script::Kawi => one!(b"kawi"),
        Script::KayahLi => one!(b"kali"),
        Script::Kharoshthi => one!(b"khar"),
        Script::KhitanSmallScript => one!(b"kits"),
        Script::Khojki => one!(b"khoj"),
        Script::Khudawadi => one!(b"sind"),
        Script::KiratRai => one!(b"krai"),
        Script::Lepcha => one!(b"lepc"),
        Script::Limbu => one!(b"limb"),
        Script::LinearA => one!(b"lina"),
        Script::LinearB => one!(b"linb"),
        Script::Lisu => one!(b"lisu"),
        Script::Lycian => one!(b"lyci"),
        Script::Lydian => one!(b"lydi"),
        Script::Mahajani => one!(b"mahj"),
        Script::Makasar => one!(b"maka"),
        Script::Mandaic => one!(b"mand"),
        Script::Manichaean => one!(b"mani"),
        Script::Marchen => one!(b"marc"),
        Script::MasaramGondi => one!(b"gonm"),
        Script::Medefaidrin => one!(b"medf"),
        Script::MeeteiMayek => one!(b"mtei"),
        Script::MendeKikakui => one!(b"mend"),
        Script::MeroiticCursive => one!(b"merc"),
        Script::MeroiticHieroglyphs => one!(b"mero"),
        Script::Miao => one!(b"plrd"),
        Script::Modi => one!(b"modi"),
        Script::Mro => one!(b"mroo"),
        Script::Multani => one!(b"mult"),
        Script::Nabataean => one!(b"nbat"),
        Script::NagMundari => one!(b"nagm"),
        Script::Nandinagari => one!(b"nand"),
        Script::Newa => one!(b"newa"),
        Script::NewTaiLue => one!(b"talu"),
        Script::Nushu => one!(b"nshu"),
        Script::NyiakengPuachueHmong => one!(b"hmnp"),
        Script::Ogham => one!(b"ogam"),
        Script::OlChiki => one!(b"olck"),
        Script::OlOnal => one!(b"onao"),
        Script::OldHungarian => one!(b"hung"),
        Script::OldItalic => one!(b"ital"),
        Script::OldNorthArabian => one!(b"narb"),
        Script::OldPermic => one!(b"perm"),
        Script::OldPersian => one!(b"xpeo"),
        Script::OldSogdian => one!(b"sogo"),
        Script::OldSouthArabian => one!(b"sarb"),
        Script::OldTurkic => one!(b"orkh"),
        Script::OldUyghur => one!(b"ougr"),
        Script::Osage => one!(b"osge"),
        Script::Osmanya => one!(b"osma"),
        Script::PahawhHmong => one!(b"hmng"),
        Script::Palmyrene => one!(b"palm"),
        Script::PauCinHau => one!(b"pauc"),
        Script::PhagsPa => one!(b"phag"),
        Script::Phoenician => one!(b"phnx"),
        Script::PsalterPahlavi => one!(b"phlp"),
        Script::Rejang => one!(b"rjng"),
        Script::Runic => one!(b"runr"),
        Script::Samaritan => one!(b"samr"),
        Script::Saurashtra => one!(b"saur"),
        Script::Sharada => one!(b"shrd"),
        Script::Shavian => one!(b"shaw"),
        Script::Siddham => one!(b"sidd"),
        Script::Sidetic => one!(b"sidt"),
        Script::SignWriting => one!(b"sgnw"),
        Script::Sogdian => one!(b"sogd"),
        Script::SoraSompeng => one!(b"sora"),
        Script::Soyombo => one!(b"soyo"),
        Script::Sundanese => one!(b"sund"),
        Script::Sunuwar => one!(b"sunu"),
        Script::SylotiNagri => one!(b"sylo"),
        Script::Tagalog => one!(b"tglg"),
        Script::Tagbanwa => one!(b"tagb"),
        Script::TaiLe => one!(b"tale"),
        Script::TaiTham => one!(b"lana"),
        Script::TaiViet => one!(b"tavt"),
        Script::TaiYo => one!(b"tayo"),
        Script::Takri => one!(b"takr"),
        Script::Tangsa => one!(b"tnsa"),
        Script::Tangut => one!(b"tang"),
        Script::Tifinagh => one!(b"tfng"),
        Script::Tirhuta => one!(b"tirh"),
        Script::Todhri => one!(b"todr"),
        Script::TolongSiki => one!(b"tols"),
        Script::Toto => one!(b"toto"),
        Script::TuluTigalari => one!(b"tutg"),
        Script::Ugaritic => one!(b"ugar"),
        Script::Vai => one!(b"vai "),
        Script::Vithkuqi => one!(b"vith"),
        Script::Wancho => one!(b"wcho"),
        Script::WarangCiti => one!(b"wara"),
        Script::Yezidi => one!(b"yezi"),
        Script::Yi => one!(b"yi  "),
        Script::ZanabazarSquare => one!(b"zanb"),
        Script::BeriaErfe => one!(b"berf"),

        // Common / Inherited / Unknown (the pseudo-scripts that carry no
        // script-specific shaping behaviour) → the OpenType Default tag.
        Script::Common | Script::Inherited | Script::Unknown => one!(b"DFLT"),
    }
}

/// A maximal run of characters resolved to a single Unicode [`Script`].
///
/// Produced by [`script_runs`]. `start` / `end` are **char** indices
/// (not byte offsets) into the slice that was itemised — half-open
/// `[start, end)`. `script` is the run's resolved script; use
/// [`ot_script_tag`] / [`ot_script_tags`] to map it to the OpenType tag
/// the shaper feeds to its `GSUB` / `GPOS` `ScriptList` lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptRun {
    /// First char index of the run (inclusive).
    pub start: usize,
    /// One-past-the-last char index of the run (exclusive).
    pub end: usize,
    /// The run's resolved Unicode script.
    pub script: Script,
}

impl ScriptRun {
    /// Number of characters in the run.
    #[must_use]
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Whether the run is empty (never produced by [`script_runs`], but
    /// makes the type a well-formed range wrapper).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// Itemise `chars` into maximal same-script [`ScriptRun`]s.
///
/// Each character's Unicode `Script` property drives the split, with the
/// two pseudo-scripts handled so punctuation and combining marks do not
/// fragment a run:
///
/// * **`Inherited`** (combining marks, variation selectors) always joins
///   the *preceding* character's resolved script — a mark never starts a
///   new run.
/// * **`Common`** (spaces, ASCII digits, most punctuation, symbols)
///   joins the preceding run's script when there is one; a leading
///   `Common` span (before any real script appears) is provisionally
///   `Common` and is **back-filled** onto the first following real
///   script, so `"123abc"` is a single Latin run rather than a `Common`
///   run followed by a Latin run.
/// * A `Common` / `Inherited` span that sits between two *different* real
///   scripts stays attached to the **preceding** script (the run that
///   was already open), so `"abcДef"` (Latin, Cyrillic) splits exactly
///   at the script change with any intervening neutral characters going
///   to the left run.
///
/// Resolution policy note: this is scribe's own conservative
/// neutral-attachment rule built on the Unicode `Script` /
/// `Script_Extensions` property data the `intl` crate supplies. It is
/// deliberately simpler than the full UAX #24 §5.1 itemisation (which
/// additionally pairs brackets and consults `Script_Extensions` to keep
/// a `Common` character with a script it shares membership in); those
/// refinements are layered on later. The output is always a complete,
/// gap-free partition of `0..chars.len()` in order.
///
/// An empty input yields an empty `Vec`.
#[must_use]
pub fn script_runs(chars: &[char]) -> Vec<ScriptRun> {
    let mut runs: Vec<ScriptRun> = Vec::new();
    if chars.is_empty() {
        return runs;
    }

    // `current` is the resolved script of the run currently being built;
    // `None` while we are still in a leading Common/Inherited span that
    // has no real script to attach to yet.
    let mut current: Option<Script> = None;
    let mut run_start = 0usize;
    // Index of the first run that still carries the provisional leading
    // Common/Inherited script and needs back-filling once a real script
    // appears. `None` once back-filled (or if the text opened with a
    // real script).
    let mut pending_leading: Option<usize> = None;

    for (i, &c) in chars.iter().enumerate() {
        let s = script(c);
        match s {
            // Inherited / Common never *force* a boundary: they extend
            // whatever run is open.
            Script::Inherited | Script::Common | Script::Unknown => {
                if current.is_none() && runs.is_empty() {
                    // Leading neutral span with no run open yet: open a
                    // provisional run we will back-fill later.
                    current = Some(Script::Common);
                    run_start = run_start.min(i);
                    pending_leading = Some(runs.len());
                }
                // else: just keep extending the open run (no boundary).
            }
            real => {
                match current {
                    // First real script seen — back-fill the provisional
                    // leading-neutral run, if any, onto it.
                    Some(Script::Common) if pending_leading.is_some() => {
                        current = Some(real);
                        pending_leading = None;
                    }
                    Some(cur) if cur == real => {
                        // Same script — keep extending.
                    }
                    Some(_) => {
                        // Real script change: close the open run at `i`
                        // and start a new one.
                        runs.push(ScriptRun {
                            start: run_start,
                            end: i,
                            script: current.unwrap(),
                        });
                        run_start = i;
                        current = Some(real);
                    }
                    None => {
                        current = Some(real);
                        run_start = run_start.min(i);
                    }
                }
            }
        }
    }

    // Flush the final open run (always present for non-empty input).
    if let Some(cur) = current {
        runs.push(ScriptRun {
            start: run_start,
            end: chars.len(),
            script: cur,
        });
    } else {
        // Pathological: every char was neutral and we never opened a
        // provisional run (cannot happen given the leading-neutral
        // branch, but keep the partition total).
        runs.push(ScriptRun {
            start: 0,
            end: chars.len(),
            script: Script::Common,
        });
    }

    runs
}

/// Convenience wrapper over [`script_runs`] that itemises a `&str`
/// directly, collecting it into a `Vec<char>` first. The returned
/// [`ScriptRun`] indices are **char** indices into that collected
/// sequence (i.e. `text.chars().nth(start)` is the run's first
/// character).
#[must_use]
pub fn script_runs_str(text: &str) -> Vec<ScriptRun> {
    let chars: Vec<char> = text.chars().collect();
    script_runs(&chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_maps_to_latn() {
        assert_eq!(ot_script_tag(Script::Latin), *b"latn");
        assert_eq!(ot_script_tags(Script::Latin), &[*b"latn"]);
    }

    #[test]
    fn indic_scripts_return_modern_then_legacy() {
        assert_eq!(ot_script_tags(Script::Devanagari), &[*b"dev2", *b"deva"]);
        assert_eq!(ot_script_tags(Script::Bengali), &[*b"bng2", *b"beng"]);
        assert_eq!(ot_script_tags(Script::Tamil), &[*b"tml2", *b"taml"]);
        assert_eq!(ot_script_tags(Script::Myanmar), &[*b"mym2", *b"mymr"]);
        // Primary tag is the modern one.
        assert_eq!(ot_script_tag(Script::Devanagari), *b"dev2");
    }

    #[test]
    fn space_padded_tags_keep_the_pad() {
        assert_eq!(ot_script_tag(Script::Lao), *b"lao ");
        assert_eq!(ot_script_tag(Script::Nko), *b"nko ");
        assert_eq!(ot_script_tag(Script::Yi), *b"yi  ");
        assert_eq!(ot_script_tag(Script::Vai), *b"vai ");
    }

    #[test]
    fn cjk_kana_share_one_tag() {
        assert_eq!(ot_script_tag(Script::Han), *b"hani");
        assert_eq!(ot_script_tag(Script::Hiragana), *b"kana");
        assert_eq!(ot_script_tag(Script::Katakana), *b"kana");
    }

    #[test]
    fn common_inherited_unknown_are_default() {
        assert_eq!(ot_script_tag(Script::Common), *b"DFLT");
        assert_eq!(ot_script_tag(Script::Inherited), *b"DFLT");
        assert_eq!(ot_script_tag(Script::Unknown), *b"DFLT");
    }

    #[test]
    fn empty_input_yields_no_runs() {
        assert!(script_runs(&[]).is_empty());
        assert!(script_runs_str("").is_empty());
    }

    #[test]
    fn pure_latin_is_one_run() {
        let runs = script_runs_str("hello");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].script, Script::Latin);
        assert_eq!((runs[0].start, runs[0].end), (0, 5));
        assert_eq!(runs[0].len(), 5);
        assert!(!runs[0].is_empty());
    }

    #[test]
    fn common_punctuation_does_not_fragment() {
        // The comma + space are Common and must stay inside the Latin
        // run rather than splitting it into three.
        let runs = script_runs_str("abc, def");
        assert_eq!(runs.len(), 1, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Latin);
        assert_eq!((runs[0].start, runs[0].end), (0, 8));
    }

    #[test]
    fn leading_digits_backfill_onto_following_script() {
        // "123abc": the digits are Common; they back-fill onto Latin so
        // the whole thing is one Latin run.
        let runs = script_runs_str("123abc");
        assert_eq!(runs.len(), 1, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Latin);
        assert_eq!((runs[0].start, runs[0].end), (0, 6));
    }

    #[test]
    fn script_change_splits_with_neutral_going_left() {
        // Latin "abc" + space (Common) + Cyrillic "Дef"? No — keep it
        // unambiguous: Latin then Cyrillic with a separating space.
        // "abc Дзе": abc = Latin, space = Common (joins Latin),
        // Дзе = Cyrillic.
        let runs = script_runs_str("abc \u{0414}\u{0437}\u{0435}");
        assert_eq!(runs.len(), 2, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Latin);
        assert_eq!((runs[0].start, runs[0].end), (0, 4)); // includes the space
        assert_eq!(runs[1].script, Script::Cyrillic);
        assert_eq!((runs[1].start, runs[1].end), (4, 7));
    }

    #[test]
    fn inherited_marks_join_preceding_script() {
        // Latin 'e' + U+0301 COMBINING ACUTE ACCENT (Inherited) stays
        // one Latin run.
        let runs = script_runs_str("e\u{0301}");
        assert_eq!(runs.len(), 1, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Latin);
        assert_eq!((runs[0].start, runs[0].end), (0, 2));
    }

    #[test]
    fn partition_is_total_and_in_order() {
        // Whatever the input, the runs must tile [0, n) with no gaps or
        // overlaps, in increasing order.
        let text = "Hello, \u{05E9}\u{05DC}\u{05D5}\u{05DD} 123 \u{4E16}\u{754C}!";
        let chars: Vec<char> = text.chars().collect();
        let runs = script_runs(&chars);
        assert!(!runs.is_empty());
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs.last().unwrap().end, chars.len());
        for w in runs.windows(2) {
            assert_eq!(w[0].end, w[1].start, "gap/overlap between runs: {runs:?}");
            assert_ne!(w[0].script, w[1].script, "adjacent runs share a script");
        }
    }

    #[test]
    fn hebrew_and_han_are_distinct_runs() {
        // Hebrew then Han, separated by a space.
        let runs = script_runs_str("\u{05D0}\u{05D1} \u{4E16}\u{754C}");
        assert_eq!(runs.len(), 2, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Hebrew);
        assert_eq!(runs[1].script, Script::Han);
    }
}
