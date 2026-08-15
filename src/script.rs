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
//! * [`ScriptRun`] / [`script_runs`] / [`resolve_scripts`] — itemise a
//!   string into maximal same-script runs, resolving the `Common` and
//!   `Inherited` pseudo-scripts onto a neighbouring real script so a run
//!   like `"abc, def"` (where the comma and space are `Common`) stays
//!   one Latin run rather than fragmenting on the punctuation. The
//!   resolution implements the full UAX #24 §5 rule set: the
//!   `Script_Extensions` (scx) constraint sets (§5.3 — U+30FC KATAKANA-
//!   HIRAGANA PROLONGED SOUND MARK continues a Kana run but *not* a
//!   Latin one), combining-mark inheritance (§5.2), and the paired-
//!   bracket refinement (§5.1 — the closing element of a bracket pair
//!   resolves to the same script as its opening partner, i.e. the
//!   *enclosing* text's script).
//!
//! The Unicode `Script` / `Script_Extensions` properties themselves are
//! supplied by the `intl` crate's compiled UCD tables
//! ([`intl::unicode::script::script`] /
//! [`intl::unicode::script::script_extensions`]); this module never
//! re-derives them. The normative basis is the staged UAX #24
//! transcription (`docs/text/unicode-script/uax24-script-extensions.md`)
//! plus the UCD data files under `docs/text/opentype/ucd/`
//! (`Scripts.txt` / `ScriptExtensions.txt`); bracket pairing reuses the
//! crate's `BidiBrackets.txt` table via [`crate::bidi::paired_bracket`].

use crate::bidi::{paired_bracket, BracketKind};
use intl::unicode::script::{script, script_extensions, Script, ScriptExtensions};

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

/// Per-character classification derived from the `Script` (sc) and
/// `Script_Extensions` (scx) properties — UAX #24 §2.1 / §3.
#[derive(Clone, Copy)]
enum CharClass {
    /// `sc = Inherited` — combining marks, ZWJ/ZWNJ, variation
    /// selectors. §5.2: never break between a mark and its base; the
    /// mark inherits the open run's script unconditionally. The scx set
    /// is kept as a *hint* for spans with no base yet (e.g. the Arabic
    /// vowel signs' `{Arab Syrc}`).
    Mark(ScriptExtensions),
    /// `sc = Common` / `Unknown` with an implicit scx set — usable with
    /// almost any script (§3.2 rule 1). Attaches to whatever run is
    /// open.
    Neutral,
    /// `sc` implicit but scx is a *limited explicit set* (§3.2 rule 2):
    /// the character continues a run only if the run's script is in the
    /// set (§5.3 — U+30FC continues Hiragana/Katakana, never Latin).
    Constrained(&'static [Script]),
    /// `sc` is an explicit script. The scx set (which contains `sc`,
    /// §3.1 rule D) may list *extra* scripts the character is borrowed
    /// into, in which case it can also continue a run of one of those.
    Determinate(Script, ScriptExtensions),
}

/// UAX #9 BD16's canonical-equivalence clause, reused for §5.1 bracket
/// pairing: U+2329/U+232A are canonically equivalent to U+3008/U+3009,
/// so an opener of one form pairs with a closer of the other.
fn canon_bracket(c: char) -> char {
    match c {
        '\u{2329}' => '\u{3008}',
        '\u{232A}' => '\u{3009}',
        other => other,
    }
}

/// Resolve every character of `chars` to one concrete Unicode
/// [`Script`], applying the UAX #24 §5 resolution rules.
///
/// This is the per-character form of [`script_runs`] (which is a simple
/// grouping of this function's output): each `Common` / `Inherited` /
/// `Unknown` character is resolved onto a real script from its context,
/// and each scx-constrained character onto a member of its
/// `Script_Extensions` set. The rules, in the order they apply to a
/// character:
///
/// * **Paired brackets (§5.1)** — the closing element of a bracket pair
///   (per the `Bidi_Paired_Bracket` property, the same `BidiBrackets.txt`
///   table [`crate::bidi::paired_bracket`] serves) resolves to the
///   **same script as its opening partner**, i.e. the *enclosing*
///   text's script — so in `"abc (αβ) def"` both parentheses resolve to
///   Latin rather than the closer picking up Greek from its left
///   neighbour. Bracket tracking uses a bounded stack (63 entries, the
///   UAX #9 BD16 depth) with the BD16 canonical-equivalence clause for
///   U+2329/U+232A ↔ U+3008/U+3009; unmatched closers degrade to plain
///   neutral attachment.
/// * **Combining marks (§5.2)** — `sc = Inherited` characters never
///   break a run: they take the open run's script unconditionally
///   (their scx set, when explicit, only narrows the resolution of a
///   span that has no base character yet).
/// * **scx constraint sets (§5.3)** — a character whose scx names a
///   limited script set continues the open run only when the run's
///   script is a member. Otherwise it opens an *ambiguous span*
///   constrained to the set; consecutive constrained characters
///   intersect their sets, and the span resolves to the first following
///   determinate script that is a member (or to the set's first script
///   when none arrives). So `"アー"` is one Katakana run, `"あー"` one
///   Hiragana run, and `"abcー"` splits into a Latin run and a Kana
///   run instead of swallowing U+30FC into Latin.
/// * **Neutrals** — implicit-scx `Common` / `Unknown` characters attach
///   to the open run; a leading neutral span back-fills onto the first
///   real script (so `"123abc"` is all Latin). Text that never resolves
///   to a real script comes back as `Common`.
///
/// The output always has exactly `chars.len()` entries, each an
/// explicit script or `Script::Common` (for fully neutral text);
/// `Inherited` / `Unknown` are never emitted.
#[must_use]
pub fn resolve_scripts(chars: &[char]) -> Vec<Script> {
    /// Resolve the whole pending span to `to` and clear it.
    fn resolve_span(
        resolved: &mut [Option<Script>],
        pending: &mut Vec<usize>,
        pending_set: &mut Option<Vec<Script>>,
        to: Script,
    ) {
        for &idx in pending.iter() {
            resolved[idx] = Some(to);
        }
        pending.clear();
        *pending_set = None;
    }

    /// Resolve the pending span to its best fallback: the first script
    /// of the constraint set, or `Common` for an unconstrained span.
    fn flush_fallback(
        resolved: &mut [Option<Script>],
        pending: &mut Vec<usize>,
        pending_set: &mut Option<Vec<Script>>,
    ) {
        if pending.is_empty() {
            *pending_set = None;
            return;
        }
        let fb = pending_set
            .as_ref()
            .and_then(|ps| ps.first().copied())
            .unwrap_or(Script::Common);
        resolve_span(resolved, pending, pending_set, fb);
    }

    let mut resolved: Vec<Option<Script>> = vec![None; chars.len()];
    // Script of the currently open run; `None` while an unresolved
    // (pending) span is being accumulated.
    let mut current: Option<Script> = None;
    // Indices awaiting resolution, plus the scx constraint the span has
    // accumulated (`None` = unconstrained: only neutrals so far).
    let mut pending: Vec<usize> = Vec::new();
    let mut pending_set: Option<Vec<Script>> = None;
    // Open-bracket stack: (canonicalised expected closer, opener index).
    let mut brackets: Vec<(char, usize)> = Vec::new();
    const BRACKET_STACK_MAX: usize = 63;

    for (i, &c) in chars.iter().enumerate() {
        let sc = script(c);
        let scx = script_extensions(c);
        let class = match sc {
            Script::Inherited => CharClass::Mark(scx),
            Script::Common | Script::Unknown => match scx {
                ScriptExtensions::Multiple(set) => CharClass::Constrained(set),
                _ => CharClass::Neutral,
            },
            real => CharClass::Determinate(real, scx),
        };

        // §5.1 paired-bracket refinement. Every `Bidi_Paired_Bracket`
        // character has an implicit or CJK-constrained Common script,
        // so only the Neutral / Constrained classes can be brackets.
        let bracket = match class {
            CharClass::Neutral | CharClass::Constrained(_) => paired_bracket(c),
            _ => None,
        };
        if let Some((_, BracketKind::Close)) = bracket {
            let key = canon_bracket(c);
            if let Some(depth) = brackets.iter().rposition(|&(cl, _)| cl == key) {
                let (_, opener_idx) = brackets[depth];
                brackets.truncate(depth);
                if let Some(s) = resolved[opener_idx] {
                    // The closing element resolves to the same script
                    // as its opening partner — the enclosing text.
                    if current != Some(s) {
                        // Close any half-open ambiguous span first: it
                        // joins the enclosing script when compatible,
                        // else falls back on its own constraint.
                        if !pending.is_empty() {
                            let compatible =
                                pending_set.as_ref().map_or(true, |ps| ps.contains(&s));
                            if compatible {
                                resolve_span(&mut resolved, &mut pending, &mut pending_set, s);
                            } else {
                                flush_fallback(&mut resolved, &mut pending, &mut pending_set);
                            }
                        }
                        current = Some(s);
                    }
                    resolved[i] = Some(s);
                } else {
                    // Opener is itself still pending: closer joins the
                    // same span and they resolve together.
                    pending.push(i);
                }
                continue;
            }
            // Unmatched closer: plain neutral / constrained handling.
        }

        match class {
            CharClass::Mark(scx) => {
                if let Some(s) = current {
                    // §5.2: the mark inherits its base's script.
                    resolved[i] = Some(s);
                } else {
                    // Base-less mark: joins the pending span. An
                    // explicit scx set narrows the span's resolution
                    // when compatible with what it already carries.
                    if let ScriptExtensions::Multiple(set) = scx {
                        match &mut pending_set {
                            None => pending_set = Some(set.to_vec()),
                            Some(ps) => {
                                let inter: Vec<Script> =
                                    ps.iter().copied().filter(|s| set.contains(s)).collect();
                                if !inter.is_empty() {
                                    *ps = inter;
                                }
                            }
                        }
                    }
                    pending.push(i);
                }
            }
            CharClass::Neutral => {
                if let Some(s) = current {
                    resolved[i] = Some(s);
                } else {
                    pending.push(i);
                }
            }
            CharClass::Constrained(set) => {
                if let Some(t) = current {
                    if set.contains(&t) {
                        // §5.3: compatible with the open run — continue.
                        resolved[i] = Some(t);
                    } else {
                        // The open run cannot absorb this character: it
                        // ends here and an ambiguous span constrained
                        // to `set` opens.
                        current = None;
                        pending.push(i);
                        pending_set = Some(set.to_vec());
                    }
                } else {
                    match &mut pending_set {
                        None => {
                            pending.push(i);
                            pending_set = Some(set.to_vec());
                        }
                        Some(ps) => {
                            let inter: Vec<Script> =
                                ps.iter().copied().filter(|s| set.contains(s)).collect();
                            if inter.is_empty() {
                                // Disjoint constraints: the old span
                                // resolves on its own fallback and a
                                // new one opens for this character.
                                flush_fallback(&mut resolved, &mut pending, &mut pending_set);
                                pending.push(i);
                                pending_set = Some(set.to_vec());
                            } else {
                                pending.push(i);
                                *ps = inter;
                            }
                        }
                    }
                    // A constraint narrowed to one script is resolved.
                    if let Some(ps) = &pending_set {
                        if ps.len() == 1 {
                            let s = ps[0];
                            resolve_span(&mut resolved, &mut pending, &mut pending_set, s);
                            current = Some(s);
                        }
                    }
                }
            }
            CharClass::Determinate(s, scx) => {
                if let Some(t) = current {
                    if t == s || scx.contains(t) {
                        // Same script, or a borrowed character whose
                        // scx lists the run's script (§3.1 note) —
                        // continue the open run.
                        resolved[i] = Some(t);
                    } else {
                        resolved[i] = Some(s);
                        current = Some(s);
                    }
                } else {
                    let compatible = pending_set.as_ref().map_or(true, |ps| ps.contains(&s));
                    if compatible {
                        // Back-fill the pending span (leading neutrals,
                        // or a constrained span this script satisfies).
                        resolve_span(&mut resolved, &mut pending, &mut pending_set, s);
                    } else {
                        flush_fallback(&mut resolved, &mut pending, &mut pending_set);
                    }
                    resolved[i] = Some(s);
                    current = Some(s);
                }
            }
        }

        // Push opening brackets *after* the attach so the closer
        // inherits the opener's own resolution.
        if let Some((partner, BracketKind::Open)) = bracket {
            if brackets.len() < BRACKET_STACK_MAX {
                brackets.push((canon_bracket(partner), i));
            }
        }
    }

    // A trailing unresolved span falls back to the first script of its
    // constraint set, or Common for fully neutral text.
    flush_fallback(&mut resolved, &mut pending, &mut pending_set);

    resolved
        .into_iter()
        .map(|s| s.unwrap_or(Script::Common))
        .collect()
}

/// Itemise `chars` into maximal same-script [`ScriptRun`]s.
///
/// Each character is first resolved to a concrete script by
/// [`resolve_scripts`] — the full UAX #24 §5 resolution: combining-mark
/// inheritance (§5.2), `Script_Extensions` constraint sets (§5.3), the
/// paired-bracket refinement (§5.1), leading-neutral back-fill, and
/// neutrals otherwise attaching to the *preceding* run. Consecutive
/// characters with the same resolved script then form one run, so:
///
/// * `"abc, def"` is one Latin run (comma + spaces are neutral);
/// * `"123abc"` is one Latin run (leading neutrals back-fill);
/// * `"abc Дзе"` splits at the script change, the space joining the
///   Latin (preceding) run;
/// * `"abc (αβ) def"` is Latin / Greek / Latin with **both** parentheses
///   in the Latin runs (§5.1);
/// * `"アー"` is a single Katakana run, and `"abcー"` splits U+30FC
///   *out* of the Latin run (§5.3).
///
/// The output is always a complete, gap-free partition of
/// `0..chars.len()` in order, with adjacent runs differing in script.
/// An empty input yields an empty `Vec`.
#[must_use]
pub fn script_runs(chars: &[char]) -> Vec<ScriptRun> {
    let mut runs: Vec<ScriptRun> = Vec::new();
    for (i, s) in resolve_scripts(chars).into_iter().enumerate() {
        match runs.last_mut() {
            Some(last) if last.script == s => last.end = i + 1,
            _ => runs.push(ScriptRun {
                start: i,
                end: i + 1,
                script: s,
            }),
        }
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

    // ---- UAX #24 §5 refinements (Script_Extensions + brackets) ----

    #[test]
    fn scx_constrained_char_continues_a_compatible_run() {
        // U+30FC KATAKANA-HIRAGANA PROLONGED SOUND MARK: sc = Common,
        // scx = {Hira Kana} (§3, the UAX #24 worked example). After a
        // Katakana character it continues the Katakana run.
        let runs = script_runs_str("\u{30A2}\u{30FC}");
        assert_eq!(runs.len(), 1, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Katakana);
        // …and after a Hiragana character, the Hiragana run.
        let runs = script_runs_str("\u{3042}\u{30FC}");
        assert_eq!(runs.len(), 1, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Hiragana);
    }

    #[test]
    fn scx_constrained_char_does_not_continue_an_incompatible_run() {
        // §5.3: U+30FC "should continue only runs of certain scripts,
        // not a Latin run". Latin ∉ {Hira Kana}, so the Latin run ends
        // and U+30FC resolves within its own scx set.
        let runs = script_runs_str("abc\u{30FC}");
        assert_eq!(runs.len(), 2, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Latin);
        assert_eq!((runs[0].start, runs[0].end), (0, 3));
        assert!(
            matches!(runs[1].script, Script::Hiragana | Script::Katakana),
            "got {runs:?}"
        );
    }

    #[test]
    fn scx_constrained_span_resolves_to_following_member_script() {
        // U+060C ARABIC COMMA: sc = Common, scx names Arabic-family
        // scripts (not Latin). Between a Latin run and an Arabic run it
        // must join the Arabic side, not attach left to Latin.
        let runs = script_runs_str("a\u{060C}\u{0628}");
        assert_eq!(runs.len(), 2, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Latin);
        assert_eq!((runs[0].start, runs[0].end), (0, 1));
        assert_eq!(runs[1].script, Script::Arabic);
        assert_eq!((runs[1].start, runs[1].end), (1, 3));
    }

    #[test]
    fn arabic_vowel_signs_inherit_their_base() {
        // U+064E ARABIC FATHA: sc = Inherited, scx = {Arab Syrc}. After
        // an Arabic base it stays in the Arabic run (§5.2)…
        let runs = script_runs_str("\u{0628}\u{064E}");
        assert_eq!(runs.len(), 1, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Arabic);
        // …and §5.2's "never break between a mark and its base" wins
        // even over the scx hint: a (degenerate) Latin base keeps its
        // mark in the Latin run.
        let runs = script_runs_str("a\u{064E}");
        assert_eq!(runs.len(), 1, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Latin);
    }

    #[test]
    fn base_less_constrained_mark_resolves_within_its_scx_set() {
        // A lone U+064E has no base: the span falls back to a member
        // of its scx set rather than Common.
        let resolved = resolve_scripts(&['\u{064E}']);
        assert!(
            matches!(resolved[0], Script::Arabic | Script::Syriac),
            "got {resolved:?}"
        );
    }

    #[test]
    fn bracket_pair_resolves_to_enclosing_script() {
        // §5.1: parentheses around a Greek word inside Latin text both
        // resolve to Latin (the enclosing script) — the closer must NOT
        // pick up Greek from its left neighbour.
        let text = "abc (\u{03A8}\u{03B1}) def";
        let chars: Vec<char> = text.chars().collect();
        let runs = script_runs(&chars);
        assert_eq!(runs.len(), 3, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Latin);
        assert_eq!(runs[1].script, Script::Greek);
        assert_eq!(runs[2].script, Script::Latin);
        // "abc (" | "Ψα" | ") def"
        assert_eq!((runs[0].start, runs[0].end), (0, 5));
        assert_eq!((runs[1].start, runs[1].end), (5, 7));
        assert_eq!((runs[2].start, runs[2].end), (7, 12));
    }

    #[test]
    fn nested_bracket_pairs_resolve_independently() {
        // "a[б(в)г]d": the inner pair encloses Cyrillic (opened while
        // the Cyrillic run was live), the outer pair Latin.
        let text = "a[\u{0431}(\u{0432})\u{0433}]d";
        let chars: Vec<char> = text.chars().collect();
        let resolved = resolve_scripts(&chars);
        assert_eq!(resolved[1], Script::Latin, "outer opener: {resolved:?}");
        assert_eq!(resolved[3], Script::Cyrillic, "inner opener: {resolved:?}");
        assert_eq!(resolved[5], Script::Cyrillic, "inner closer: {resolved:?}");
        assert_eq!(resolved[7], Script::Latin, "outer closer: {resolved:?}");
    }

    #[test]
    fn unmatched_closer_attaches_like_a_neutral() {
        // A stray ")" with no opener keeps the old left-attachment.
        let runs = script_runs_str("abc) \u{0431}");
        assert_eq!(runs.len(), 2, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Latin);
        assert_eq!((runs[0].start, runs[0].end), (0, 5));
        assert_eq!(runs[1].script, Script::Cyrillic);
    }

    #[test]
    fn canonically_equivalent_angle_brackets_pair() {
        // BD16 canonical equivalence: U+2329 opener pairs with U+3009
        // closer. Enclosing script is Latin.
        let text = "a\u{2329}\u{0431}\u{3009}b";
        let chars: Vec<char> = text.chars().collect();
        let resolved = resolve_scripts(&chars);
        assert_eq!(resolved[1], Script::Latin, "{resolved:?}");
        assert_eq!(resolved[3], Script::Latin, "{resolved:?}");
    }

    #[test]
    fn devanagari_danda_continues_a_devanagari_run() {
        // U+0964 DEVANAGARI DANDA: sc = Common, scx spans the Indic
        // scripts — it continues a Devanagari run.
        let runs = script_runs_str("\u{0915}\u{0964}");
        assert_eq!(runs.len(), 1, "got {runs:?}");
        assert_eq!(runs[0].script, Script::Devanagari);
    }

    #[test]
    fn resolve_scripts_never_emits_pseudo_scripts_for_real_text() {
        let chars: Vec<char> = "Hello, \u{05E9}\u{05DC} 123 \u{4E16}!".chars().collect();
        for (i, s) in resolve_scripts(&chars).into_iter().enumerate() {
            assert!(
                !matches!(s, Script::Inherited | Script::Unknown),
                "char {i} resolved to pseudo-script {s:?}"
            );
        }
    }

    #[test]
    fn resolve_scripts_length_matches_input() {
        let chars: Vec<char> = "a(\u{03B1}\u{060C})\u{30FC} \u{0964}".chars().collect();
        assert_eq!(resolve_scripts(&chars).len(), chars.len());
        assert!(resolve_scripts(&[]).is_empty());
    }
}
