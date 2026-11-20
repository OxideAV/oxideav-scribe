//! Shaping conformance harness.
//!
//! Every case shapes one string with one font and compares the glyph
//! ids, the absolute glyph positions and the run advance against an
//! expected glyph run.
//!
//! * For the fonts committed under `tests/fixtures/` the expected runs
//!   live in `tests/fixtures/shaping/expected.tsv`, so the comparison
//!   runs everywhere (CI included) with no external tool.
//! * When the `hb-shape` executable is on `PATH` the same cases are
//!   also shaped by it as a black-box reference, and so are the cases
//!   that need a system font or one of the network-gated fixtures
//!   (complex scripts, CJK, emoji). A case whose font or reference
//!   tool is absent is skipped at run time.
//!
//! Environment:
//!
//! * `SCRIBE_SHAPING_REPORT=1` — print every case with its verdict.
//! * `SCRIBE_SHAPING_REGEN=1` — rewrite `expected.tsv` from the
//!   reference tool (fixture provenance: `hb-shape`, run with
//!   `--no-glyph-names` and the options recorded per line).
//!
//! Positions are compared as absolute pen-relative coordinates in font
//! design units (the shaper is run at `size_px == units_per_em`), so
//! two runs that draw every glyph at the same place agree even when
//! they split a displacement differently between offset and advance.

use oxideav_scribe::{Face, FaceChain};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where a font may live. Fixture fonts are relative to
/// `tests/fixtures/`; the others are probed in order.
struct FontSource {
    key: &'static str,
    paths: &'static [&'static str],
    index: u32,
    fixture: bool,
}

const FONTS: &[FontSource] = &[
    FontSource {
        key: "dejavu",
        paths: &["DejaVuSans.ttf"],
        index: 0,
        fixture: true,
    },
    FontSource {
        key: "inter",
        paths: &["InterVariable.ttf"],
        index: 0,
        fixture: true,
    },
    FontSource {
        key: "sourcesans",
        paths: &["SourceSans3-Regular.otf"],
        index: 0,
        fixture: true,
    },
    FontSource {
        key: "arial",
        paths: &[
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/usr/share/fonts/truetype/msttcorefonts/Arial.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
        ],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "times",
        paths: &[
            "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
            "/usr/share/fonts/truetype/msttcorefonts/Times_New_Roman.ttf",
            "C:\\Windows\\Fonts\\times.ttf",
        ],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "tahoma",
        paths: &[
            "/System/Library/Fonts/Supplemental/Tahoma.ttf",
            "C:\\Windows\\Fonts\\tahoma.ttf",
        ],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "sfarabic",
        paths: &["/System/Library/Fonts/SFArabic.ttf"],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "notoarabic",
        paths: &["NotoSansArabic-Regular.ttf"],
        index: 0,
        fixture: true,
    },
    FontSource {
        key: "sfhebrew",
        paths: &["/System/Library/Fonts/SFHebrew.ttf"],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "kohinoor",
        paths: &["/System/Library/Fonts/Kohinoor.ttc"],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "notodeva",
        paths: &[
            "/usr/share/fonts/truetype/noto/NotoSansDevanagari-Regular.ttf",
            "/usr/share/fonts/noto/NotoSansDevanagari-Regular.ttf",
        ],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "kohinoorbangla",
        paths: &["/System/Library/Fonts/KohinoorBangla.ttc"],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "notobengali",
        paths: &[
            "/usr/share/fonts/truetype/noto/NotoSansBengali-Regular.ttf",
            "/usr/share/fonts/noto/NotoSansBengali-Regular.ttf",
        ],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "notokannada",
        paths: &["/System/Library/Fonts/NotoSansKannada.ttc"],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "thonburi",
        paths: &["/System/Library/Fonts/Supplemental/Thonburi.ttc"],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "notothai",
        paths: &[
            "/usr/share/fonts/truetype/noto/NotoSansThai-Regular.ttf",
            "/usr/share/fonts/noto/NotoSansThai-Regular.ttf",
        ],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "arialunicode",
        paths: &[
            "/Library/Fonts/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        ],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "sdgothic",
        paths: &["/System/Library/Fonts/AppleSDGothicNeo.ttc"],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "hiragino",
        paths: &["/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc"],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "notocjk",
        paths: &[
            "cache:NotoSansCJK-Medium.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        ],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "appleemoji",
        paths: &["/System/Library/Fonts/Apple Color Emoji.ttc"],
        index: 0,
        fixture: false,
    },
    FontSource {
        key: "notoemoji",
        paths: &[
            "cache:NotoColorEmoji.ttf",
            "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
            "/usr/share/fonts/noto/NotoColorEmoji.ttf",
        ],
        index: 0,
        fixture: false,
    },
];

/// One shaping case: `name`, the text, the extra features the caller
/// switches on, whether the run is vertical, and the fonts to try.
struct Case {
    name: &'static str,
    text: &'static str,
    features: &'static [&'static str],
    vertical: bool,
    /// Variation axis settings (`(tag, value)`), for variable fonts.
    variations: &'static [(&'static str, f32)],
    fonts: &'static [&'static str],
}

const fn case(name: &'static str, text: &'static str, fonts: &'static [&'static str]) -> Case {
    Case {
        name,
        text,
        features: &[],
        vertical: false,
        variations: &[],
        fonts,
    }
}

const fn var(
    name: &'static str,
    text: &'static str,
    variations: &'static [(&'static str, f32)],
    fonts: &'static [&'static str],
) -> Case {
    Case {
        name,
        text,
        features: &[],
        vertical: false,
        variations,
        fonts,
    }
}

const fn feat(
    name: &'static str,
    text: &'static str,
    features: &'static [&'static str],
    fonts: &'static [&'static str],
) -> Case {
    Case {
        name,
        text,
        features,
        vertical: false,
        variations: &[],
        fonts,
    }
}

const fn vert(name: &'static str, text: &'static str, fonts: &'static [&'static str]) -> Case {
    Case {
        name,
        text,
        features: &[],
        vertical: true,
        variations: &[],
        fonts,
    }
}

const LATIN: &[&str] = &["dejavu", "inter", "sourcesans", "arial", "times"];
const ARABIC: &[&str] = &[
    "dejavu",
    "arial",
    "times",
    "tahoma",
    "sfarabic",
    "notoarabic",
];
const HEBREW: &[&str] = &["dejavu", "arial", "times", "sfhebrew"];
const DEVA: &[&str] = &["kohinoor", "notodeva", "arialunicode"];
const BENGALI: &[&str] = &["kohinoorbangla", "notobengali"];
const KANNADA: &[&str] = &["notokannada"];
const THAI: &[&str] = &["thonburi", "notothai", "tahoma", "arialunicode"];
const HANGUL: &[&str] = &["sdgothic", "notocjk", "arialunicode"];
const CJK: &[&str] = &["hiragino", "notocjk"];
const EMOJI: &[&str] = &["appleemoji", "notoemoji"];

const CASES: &[Case] = &[
    // ---- Latin ------------------------------------------------------
    case("latin-plain", "Hello, world!", LATIN),
    case(
        "latin-ligatures",
        "office affluent fjord fi fl ffi ffl",
        LATIN,
    ),
    case("latin-kerning", "AVATAR Type To. WAVE Yo, LT r.", LATIN),
    case(
        "latin-pangram",
        "The quick brown fox jumps over the lazy dog.",
        LATIN,
    ),
    case(
        "latin-digits",
        "0123456789 1/2 3/4 10:45 (x-y) [a+b]",
        LATIN,
    ),
    case(
        "latin-precomposed",
        "\u{e9}\u{e8}\u{ea}\u{eb} \u{f1} \u{fc} \u{e5} \u{c7}\u{e7} \u{1ea5}\u{1ed9}",
        LATIN,
    ),
    case(
        "latin-combining",
        "e\u{301} a\u{300} o\u{302} u\u{308} n\u{303} A\u{30a} c\u{327}",
        LATIN,
    ),
    case(
        "latin-mark-stack",
        "a\u{302}\u{301} o\u{308}\u{304} e\u{323}\u{302} q\u{307}\u{323}",
        LATIN,
    ),
    case(
        "latin-mark-on-mark",
        "q\u{302}\u{301} Q\u{301}\u{301} x\u{323}\u{331}",
        LATIN,
    ),
    case(
        "latin-mark-on-ligature",
        "fi\u{301} f\u{301}i ffl\u{323}",
        LATIN,
    ),
    case(
        "latin-cyrillic-greek",
        "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442} \u{393}\u{3b5}\u{3b9}\u{3ac} \u{3c3}\u{3bf}\u{3c5}",
        LATIN,
    ),
    case(
        "latin-zwj-zwnj",
        "f\u{200c}i f\u{200d}i of\u{200c}fice",
        LATIN,
    ),
    case(
        "latin-variation-selector",
        "a\u{fe00}b 0\u{fe00} \u{2269}\u{fe00}",
        LATIN,
    ),
    case("latin-spaces", "a\u{a0}b\u{2009}c\u{2003}d  e\tf", LATIN),
    feat("latin-smallcaps", "Small Caps 123", &["smcp"], LATIN),
    feat(
        "latin-caps-to-smallcaps",
        "NASA and Unicode",
        &["c2sc", "smcp"],
        LATIN,
    ),
    feat("latin-oldstyle-figures", "0123456789", &["onum"], LATIN),
    feat("latin-tabular-figures", "0123456789 1111", &["tnum"], LATIN),
    feat("latin-fractions", "1/2 3/4 22/7", &["frac"], LATIN),
    feat("latin-superscript", "x2 H2O 1st", &["sups"], LATIN),
    feat("latin-slashed-zero", "O0 1000", &["zero"], LATIN),
    feat("latin-discretionary", "ct st Th act", &["dlig"], LATIN),
    feat(
        "latin-stylistic-set",
        "a g l 1 4 6 9",
        &["ss01", "ss02"],
        LATIN,
    ),
    feat("latin-case", "(A-B) [C:D] \u{bf}Q?", &["case"], LATIN),
    feat("latin-no-ligatures", "office fi fl", &["-liga"], LATIN),
    feat("latin-no-kerning", "AVATAR To.", &["-kern"], LATIN),
    var(
        "latin-variable-bold",
        "AVATAR Type To. office",
        &[("wght", 700.0)],
        &["inter"],
    ),
    var(
        "latin-variable-thin",
        "AVATAR Type To. office",
        &[("wght", 100.0)],
        &["inter"],
    ),
    var(
        "latin-variable-optical",
        "AVATAR Type To. office",
        &[("opsz", 32.0), ("wght", 550.0)],
        &["inter"],
    ),
    var(
        "latin-variable-marks",
        "q\u{301} e\u{302}\u{323} fi\u{301}",
        &[("wght", 900.0)],
        &["inter"],
    ),
    // ---- Arabic -----------------------------------------------------
    case("arabic-word", "\u{633}\u{644}\u{627}\u{645}", ARABIC),
    case(
        "arabic-phrase",
        "\u{627}\u{644}\u{633}\u{644}\u{627}\u{645} \u{639}\u{644}\u{64a}\u{643}\u{645}",
        ARABIC,
    ),
    case(
        "arabic-forms",
        "\u{628} \u{628}\u{628} \u{628}\u{628}\u{628} \u{647}\u{647}\u{647} \u{639}\u{639}\u{639}",
        ARABIC,
    ),
    case(
        "arabic-non-joiners",
        "\u{62f}\u{631}\u{633} \u{648}\u{631}\u{62f} \u{623}\u{62f}\u{628}",
        ARABIC,
    ),
    case(
        "arabic-lam-alef",
        "\u{644}\u{627} \u{644}\u{623} \u{644}\u{625} \u{644}\u{622} \u{628}\u{644}\u{627}",
        ARABIC,
    ),
    case(
        "arabic-allah",
        "\u{627}\u{644}\u{644}\u{647} \u{644}\u{644}\u{647}",
        ARABIC,
    ),
    case(
        "arabic-marks",
        "\u{628}\u{64e}\u{62a}\u{64f}\u{62b}\u{650} \u{645}\u{64f}\u{62d}\u{64e}\u{645}\u{651}\u{64e}\u{62f}\u{652}",
        ARABIC,
    ),
    case(
        "arabic-shadda-stack",
        "\u{628}\u{651}\u{64e} \u{628}\u{651}\u{650} \u{644}\u{651}\u{64b}\u{627}",
        ARABIC,
    ),
    case(
        "arabic-marks-on-lam-alef",
        "\u{644}\u{64e}\u{627} \u{644}\u{627}\u{64b} \u{644}\u{651}\u{64e}\u{627}\u{654}",
        ARABIC,
    ),
    case(
        "arabic-kashida",
        "\u{628}\u{640}\u{640}\u{628} \u{633}\u{640}\u{644}\u{640}\u{627}\u{645}",
        ARABIC,
    ),
    case(
        "arabic-zwj-zwnj",
        "\u{628}\u{200d} \u{200d}\u{628} \u{628}\u{200c}\u{628} \u{647}\u{200d}",
        ARABIC,
    ),
    case(
        "arabic-persian-urdu",
        "\u{67e}\u{686}\u{698}\u{6af} \u{6a9}\u{6cc} \u{6c1}\u{6d2} \u{679}\u{688}",
        ARABIC,
    ),
    case(
        "arabic-digits",
        "\u{661}\u{662}\u{663} \u{6f4}\u{6f5}\u{6f6} 123",
        ARABIC,
    ),
    case(
        "arabic-punctuation",
        "\u{645}\u{631}\u{62d}\u{628}\u{627}\u{60c} (\u{639}\u{627}\u{644}\u{645})\u{61f}",
        ARABIC,
    ),
    case(
        "arabic-sentence",
        "\u{647}\u{630}\u{627} \u{646}\u{635} \u{639}\u{631}\u{628}\u{64a} \u{644}\u{644}\u{627}\u{62e}\u{62a}\u{628}\u{627}\u{631}",
        ARABIC,
    ),
    // ---- Hebrew -----------------------------------------------------
    case("hebrew-word", "\u{5e9}\u{5dc}\u{5d5}\u{5dd}", HEBREW),
    case(
        "hebrew-phrase",
        "\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e2}\u{5d5}\u{5dc}\u{5dd}",
        HEBREW,
    ),
    case(
        "hebrew-points",
        "\u{5e9}\u{5c1}\u{5b8}\u{5dc}\u{5d5}\u{5b9}\u{5dd} \u{5d1}\u{5bc}\u{5b0}\u{5e8}\u{5b5}\u{5d0}",
        HEBREW,
    ),
    case(
        "hebrew-dagesh",
        "\u{5d1}\u{5bc} \u{5db}\u{5bc} \u{5e4}\u{5bc} \u{5e9}\u{5c2}",
        HEBREW,
    ),
    case(
        "hebrew-finals-punct",
        "\u{5de}\u{5dc}\u{5da}, \u{5d0}\u{5e8}\u{5e5}! (\u{5e1}\u{5d5}\u{5e3})",
        HEBREW,
    ),
    case("hebrew-presentation", "\u{fb2a}\u{fb31}\u{fb4b}", HEBREW),
    // ---- Devanagari -------------------------------------------------
    case(
        "deva-word",
        "\u{928}\u{92e}\u{938}\u{94d}\u{924}\u{947}",
        DEVA,
    ),
    case(
        "deva-hindi",
        "\u{939}\u{93f}\u{928}\u{94d}\u{926}\u{940}",
        DEVA,
    ),
    case(
        "deva-pre-base-matra",
        "\u{915}\u{93f} \u{924}\u{93f} \u{938}\u{94d}\u{925}\u{93f}",
        DEVA,
    ),
    case(
        "deva-conjuncts",
        "\u{915}\u{94d}\u{937} \u{924}\u{94d}\u{930} \u{91c}\u{94d}\u{91e} \u{936}\u{94d}\u{930} \u{926}\u{94d}\u{927}",
        DEVA,
    ),
    case(
        "deva-reph",
        "\u{930}\u{94d}\u{915} \u{915}\u{930}\u{94d}\u{92e} \u{927}\u{930}\u{94d}\u{92e}\u{93f}",
        DEVA,
    ),
    case(
        "deva-rakar",
        "\u{915}\u{94d}\u{930} \u{92a}\u{94d}\u{930} \u{91f}\u{94d}\u{930} \u{926}\u{94d}\u{930}",
        DEVA,
    ),
    case(
        "deva-half-forms",
        "\u{938}\u{94d}\u{924} \u{928}\u{94d}\u{926} \u{92e}\u{94d}\u{92c} \u{938}\u{94d}\u{925}",
        DEVA,
    ),
    case(
        "deva-matras",
        "\u{915}\u{93e} \u{915}\u{940} \u{915}\u{941} \u{915}\u{942} \u{915}\u{947} \u{915}\u{948} \u{915}\u{94b} \u{915}\u{94c} \u{915}\u{943}",
        DEVA,
    ),
    case(
        "deva-marks",
        "\u{915}\u{902} \u{915}\u{901} \u{915}\u{903} \u{915}\u{93c} \u{915}\u{93f}\u{902} \u{915}\u{940}\u{902}",
        DEVA,
    ),
    case(
        "deva-explicit-halant",
        "\u{915}\u{94d} \u{915}\u{94d}\u{200c}\u{937} \u{915}\u{94d}\u{200d}\u{937}",
        DEVA,
    ),
    case(
        "deva-sentence",
        "\u{92f}\u{939} \u{90f}\u{915} \u{92a}\u{930}\u{940}\u{915}\u{94d}\u{937}\u{923} \u{935}\u{93e}\u{915}\u{94d}\u{92f} \u{939}\u{948}\u{964}",
        DEVA,
    ),
    // ---- Bengali ----------------------------------------------------
    case(
        "bengali-word",
        "\u{9ac}\u{9be}\u{982}\u{9b2}\u{9be}",
        BENGALI,
    ),
    case(
        "bengali-pre-base",
        "\u{995}\u{9bf} \u{995}\u{9c7} \u{995}\u{9c8}",
        BENGALI,
    ),
    case(
        "bengali-split-matra",
        "\u{995}\u{9cb} \u{995}\u{9cc}",
        BENGALI,
    ),
    case(
        "bengali-conjuncts",
        "\u{995}\u{9cd}\u{9b7} \u{9a8}\u{9cd}\u{9a4} \u{9b8}\u{9cd}\u{9a5} \u{9a4}\u{9cd}\u{9b0}",
        BENGALI,
    ),
    case(
        "bengali-reph-yaphala",
        "\u{9b0}\u{9cd}\u{995} \u{995}\u{9cd}\u{9af} \u{9b0}\u{9cd}\u{9af}",
        BENGALI,
    ),
    // ---- Kannada ----------------------------------------------------
    case(
        "kannada-word",
        "\u{c95}\u{ca8}\u{ccd}\u{ca8}\u{ca1}",
        KANNADA,
    ),
    case(
        "kannada-matras",
        "\u{c95}\u{cbf} \u{c95}\u{cc6} \u{c95}\u{cca} \u{c95}\u{ccb}",
        KANNADA,
    ),
    case(
        "kannada-conjuncts",
        "\u{c95}\u{ccd}\u{cb7} \u{cb8}\u{ccd}\u{ca4}\u{ccd}\u{cb0} \u{cb0}\u{ccd}\u{c95}",
        KANNADA,
    ),
    // ---- Thai -------------------------------------------------------
    case(
        "thai-word",
        "\u{e2a}\u{e27}\u{e31}\u{e2a}\u{e14}\u{e35}",
        THAI,
    ),
    case(
        "thai-tone-marks",
        "\u{e01}\u{e48} \u{e01}\u{e49} \u{e01}\u{e34}\u{e48} \u{e01}\u{e35}\u{e49} \u{e1b}\u{e48} \u{e1b}\u{e34}\u{e49}",
        THAI,
    ),
    case(
        "thai-below-vowels",
        "\u{e01}\u{e38} \u{e01}\u{e39} \u{e0e}\u{e38} \u{e10}\u{e39} \u{e0d}\u{e38}",
        THAI,
    ),
    case(
        "thai-sara-am",
        "\u{e19}\u{e33} \u{e19}\u{e49}\u{e33} \u{e1b}\u{e33}",
        THAI,
    ),
    case(
        "thai-leading-vowels",
        "\u{e40}\u{e01} \u{e41}\u{e01} \u{e42}\u{e01} \u{e44}\u{e01} \u{e43}\u{e01}",
        THAI,
    ),
    case(
        "thai-sentence",
        "\u{e20}\u{e32}\u{e29}\u{e32}\u{e44}\u{e17}\u{e22}\u{e17}\u{e35}\u{e48}\u{e19}\u{e35}\u{e48}",
        THAI,
    ),
    // ---- Hangul -----------------------------------------------------
    case(
        "hangul-syllables",
        "\u{d55c}\u{ae00} \u{c548}\u{b155}",
        HANGUL,
    ),
    case(
        "hangul-jamo-lv",
        "\u{1112}\u{1161} \u{1100}\u{1161}",
        HANGUL,
    ),
    case(
        "hangul-jamo-lvt",
        "\u{1112}\u{1161}\u{11ab} \u{1100}\u{1173}\u{11af}",
        HANGUL,
    ),
    case(
        "hangul-syllable-plus-t",
        "\u{d558}\u{11ab} \u{ac00}\u{11a8}",
        HANGUL,
    ),
    case("hangul-compat-jamo", "\u{3131}\u{314f}\u{3134}", HANGUL),
    // ---- CJK --------------------------------------------------------
    case(
        "cjk-horizontal",
        "\u{65e5}\u{672c}\u{8a9e}\u{3001}\u{6f22}\u{5b57}\u{3002}",
        CJK,
    ),
    case(
        "cjk-kana",
        "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}\u{30fc}\u{30ab}\u{30bf}\u{30ab}\u{30ca}",
        CJK,
    ),
    case(
        "cjk-ideographic-vs",
        "\u{845b}\u{e0100} \u{8fbb}\u{e0100}",
        CJK,
    ),
    vert(
        "cjk-vertical",
        "\u{65e5}\u{672c}\u{8a9e}\u{3001}\u{6f22}\u{5b57}\u{3002}",
        CJK,
    ),
    vert(
        "cjk-vertical-punct",
        "\u{300c}\u{30fc}\u{300d}\u{ff08}\u{2026}\u{ff09}\u{3041}\u{30c3}",
        CJK,
    ),
    vert("cjk-vertical-latin", "A1\u{65e5}", CJK),
    // ---- Emoji ------------------------------------------------------
    case("emoji-plain", "\u{1f600}\u{1f44d}\u{2764}", EMOJI),
    case(
        "emoji-presentation-vs",
        "\u{2764}\u{fe0f} \u{2764}\u{fe0e} \u{263a}\u{fe0f}",
        EMOJI,
    ),
    case(
        "emoji-skin-tone",
        "\u{1f44d}\u{1f3fd} \u{1f44b}\u{1f3fb} \u{1f9d1}\u{1f3ff}",
        EMOJI,
    ),
    case(
        "emoji-zwj-family",
        "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}",
        EMOJI,
    ),
    case(
        "emoji-zwj-profession",
        "\u{1f469}\u{200d}\u{1f4bb} \u{1f468}\u{1f3fd}\u{200d}\u{1f680}",
        EMOJI,
    ),
    case(
        "emoji-flags",
        "\u{1f1ef}\u{1f1f5}\u{1f1eb}\u{1f1f7} \u{1f1fa}\u{1f1f8}",
        EMOJI,
    ),
    case("emoji-keycap", "1\u{fe0f}\u{20e3} #\u{fe0f}\u{20e3}", EMOJI),
    case(
        "emoji-tag-sequence",
        "\u{1f3f4}\u{e0067}\u{e0062}\u{e0073}\u{e0063}\u{e0074}\u{e007f}",
        EMOJI,
    ),
];

/// One glyph of a run, reduced to what a renderer sees: the glyph and
/// where its origin lands relative to the run origin (font units, Y
/// up).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placed {
    gid: u32,
    x: f32,
    y: f32,
    /// The glyph does not move the pen (a mark, an invisible glyph).
    stacked: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Run {
    glyphs: Vec<Placed>,
    advance: (f32, f32),
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn resolve(font: &FontSource) -> Option<PathBuf> {
    for p in font.paths {
        let path = if font.fixture {
            fixtures_dir().join(p)
        } else if let Some(name) = p.strip_prefix("cache:") {
            // The download cache of the network-gated fixtures (see
            // `tests/font_fixtures/`); present once those tests ran
            // with `OXIDEAV_NETWORK_TESTS=1`.
            std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
                .join("test-fixtures/fonts")
                .join(name)
        } else {
            PathBuf::from(p)
        };
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn load_face(path: &Path, index: u32) -> Option<Face> {
    let bytes = std::fs::read(path).ok()?;
    let face = if bytes.starts_with(b"ttcf") {
        Face::from_ttc_bytes(bytes, index)
    } else if bytes.starts_with(b"OTTO") {
        Face::from_otf_bytes(bytes)
    } else {
        Face::from_ttf_bytes(bytes)
    };
    face.ok()
}

/// `true` for a font whose shaping lives in Apple Advanced Typography
/// tables (`morx`) with no `GSUB` to fall back on: the reference tool
/// shapes those with a different engine, outside this crate's scope.
fn lacks_opentype_layout(path: &Path, index: u32) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let be32 = |o: usize| {
        bytes
            .get(o..o + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let start = if bytes.starts_with(b"ttcf") {
        match be32(12 + 4 * index as usize) {
            Some(o) => o,
            None => return false,
        }
    } else {
        0
    };
    let Some(count) = bytes
        .get(start + 4..start + 6)
        .map(|b| u16::from_be_bytes([b[0], b[1]]) as usize)
    else {
        return false;
    };
    let has = |tag: &[u8; 4]| {
        (0..count).any(|i| bytes.get(start + 12 + 16 * i..start + 16 + 16 * i) == Some(&tag[..]))
    };
    has(b"morx") && !has(b"GSUB")
}

fn reference_tool() -> bool {
    Command::new("hb-shape")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn unicodes(text: &str) -> String {
    text.chars()
        .map(|c| format!("U+{:04X}", c as u32))
        .collect::<Vec<_>>()
        .join(",")
}

/// Run the reference tool; returns its raw `[gid=cluster@dx,dy+adv|…]`
/// line.
fn reference_shape(path: &Path, index: u32, case: &Case) -> Option<String> {
    let mut cmd = Command::new("hb-shape");
    cmd.arg("--no-glyph-names")
        .arg(format!("--face-index={index}"))
        .arg(format!("--unicodes={}", unicodes(case.text)));
    if !case.features.is_empty() {
        cmd.arg(format!("--features={}", case.features.join(",")));
    }
    if case.vertical {
        cmd.arg("--direction=ttb");
    }
    if !case.variations.is_empty() {
        let v: Vec<String> = case
            .variations
            .iter()
            .map(|(t, v)| format!("{t}={v}"))
            .collect();
        cmd.arg(format!("--variations={}", v.join(",")));
    }
    cmd.arg(path);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Parse `[gid=cluster@dx,dy+ax,ay|…]` into absolute placements.
fn parse_reference(line: &str) -> Option<Run> {
    let body = line.trim().strip_prefix('[')?.strip_suffix(']')?;
    let mut pen = (0.0f32, 0.0f32);
    let mut glyphs = Vec::new();
    if body.is_empty() {
        return Some(Run {
            glyphs,
            advance: pen,
        });
    }
    for item in body.split('|') {
        let (gid, rest) = item.split_once('=')?;
        let gid: u32 = gid.parse().ok()?;
        let (before_adv, adv) = match rest.split_once('+') {
            Some((a, b)) => (a, Some(b)),
            None => (rest, None),
        };
        let (dx, dy) = match before_adv.split_once('@') {
            Some((_, off)) => {
                let (x, y) = off.split_once(',')?;
                (x.parse::<f32>().ok()?, y.parse::<f32>().ok()?)
            }
            None => (0.0, 0.0),
        };
        let (ax, ay) = match adv {
            Some(a) => match a.split_once(',') {
                Some((x, y)) => (x.parse::<f32>().ok()?, y.parse::<f32>().ok()?),
                None => (a.parse::<f32>().ok()?, 0.0),
            },
            None => (0.0, 0.0),
        };
        glyphs.push(Placed {
            gid,
            x: pen.0 + dx,
            y: pen.1 + dy,
            stacked: ax == 0.0 && ay == 0.0,
        });
        pen.0 += ax;
        pen.1 += ay;
    }
    Some(Run {
        glyphs,
        advance: pen,
    })
}

fn tag(s: &str) -> [u8; 4] {
    let b = s.as_bytes();
    [b[0], b[1], b[2], b[3]]
}

/// Shape with this crate at one pixel per font unit.
fn scribe_shape(chain: &FaceChain, case: &Case) -> Result<Run, String> {
    let upem = f32::from(chain.primary().units_per_em());
    let mut opts = oxideav_scribe::ShapeOptions::new();
    for f in case.features {
        opts = match f.strip_prefix('-') {
            Some(off) => opts.with_feature(tag(off), false),
            None => opts.with_feature(tag(f), true),
        };
    }
    if case.vertical {
        opts = opts.with_vertical(true);
    }
    // The reference tool shapes the whole string as one run in the
    // direction of its script (no bidi analysis); do the same.
    let rtl = case
        .text
        .chars()
        .any(|c| matches!(c as u32, 0x590..=0x8FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF));
    let line = chain
        .shape_visual_run(case.text, upem, &opts, rtl)
        .map_err(|e| format!("shape error: {e}"))?;
    let mut pen = (0.0f32, 0.0f32);
    let mut glyphs = Vec::new();
    for g in &line {
        glyphs.push(Placed {
            gid: u32::from(g.glyph_id),
            x: pen.0 + g.x_offset,
            // The crate's offsets are Y-down; the reference is Y-up.
            y: pen.1 - g.y_offset,
            stacked: g.x_advance == 0.0 && g.y_advance == 0.0,
        });
        pen.0 += g.x_advance;
        pen.1 -= g.y_advance;
    }
    Ok(Run {
        glyphs,
        advance: pen,
    })
}

/// Glyphs stacked at one pen position (several marks on a base) may be
/// listed in any order without changing what is drawn: put each such
/// group in a fixed order.
fn canonical(run: &Run) -> Run {
    let mut glyphs = run.glyphs.clone();
    let mut i = 0;
    while i < glyphs.len() {
        if !glyphs[i].stacked {
            i += 1;
            continue;
        }
        let start = i;
        while i < glyphs.len() && glyphs[i].stacked {
            i += 1;
        }
        glyphs[start..i].sort_by_key(|g| (g.gid, g.x as i64, g.y as i64));
    }
    Run {
        glyphs,
        advance: run.advance,
    }
}

/// `None` when the runs agree, else a one-line description. `space`
/// is the font's space glyph: with no advance it stands for an
/// invisible character, and where exactly nothing is drawn does not
/// matter.
fn difference(got: &Run, want: &Run, space: u32) -> Option<String> {
    const TOLERANCE: f32 = 1.5;
    let (got, want) = (&canonical(got), &canonical(want));
    let ids = |r: &Run| r.glyphs.iter().map(|g| g.gid).collect::<Vec<_>>();
    if ids(got) != ids(want) {
        return Some(format!("glyphs {:?} != {:?}", ids(got), ids(want)));
    }
    for (i, (g, w)) in got.glyphs.iter().zip(&want.glyphs).enumerate() {
        if g.stacked && w.stacked && g.gid == space {
            continue;
        }
        if (g.x - w.x).abs() > TOLERANCE || (g.y - w.y).abs() > TOLERANCE {
            return Some(format!(
                "glyph #{i} (gid {}) at ({}, {}) != ({}, {})",
                g.gid, g.x, g.y, w.x, w.y
            ));
        }
    }
    if (got.advance.0 - want.advance.0).abs() > TOLERANCE
        || (got.advance.1 - want.advance.1).abs() > TOLERANCE
    {
        return Some(format!("advance {:?} != {:?}", got.advance, want.advance));
    }
    None
}

fn expected_path() -> PathBuf {
    fixtures_dir().join("shaping/expected.tsv")
}

/// `font key \t case name \t code points \t reference line`.
fn load_expected() -> BTreeMap<(String, String), (String, String)> {
    let mut map = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(expected_path()) else {
        return map;
    };
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() == 4 {
            map.insert(
                (cols[0].to_string(), cols[1].to_string()),
                (cols[2].to_string(), cols[3].to_string()),
            );
        }
    }
    map
}

/// Known deviations: `(font key, case name)` pairs whose mismatch is
/// understood and tracked in the README's conformance section. A case
/// listed here that starts to agree fails the test, so the list cannot
/// go stale.
const KNOWN_DEVIATIONS: &[(&str, &str)] = &[
    // Digits forced into a right-to-left run (the reference tool does
    // no bidi analysis): their pair kerning is authored as an advance
    // change only, which the font parser's pair query cannot tell
    // from the placement-plus-advance records of right-to-left
    // letters. Real text puts digits in left-to-right runs.
    ("sfarabic", "arabic-digits"),
    // The font's `abvs` contextual lookup (type 5 with a
    // mark-attachment-class filter, calling a ligature lookup) is not
    // applied by the font parser.
    ("notokannada", "kannada-matras"),
    // Two marks on one base in a font that lists its mark-to-mark
    // lookup *before* its mark-to-base lookups. Applied literally in
    // that order the second mark's stacking is overwritten and both
    // marks land on the same anchor, overlapping; this crate applies
    // mark-to-mark after mark-to-base (the documented feature order),
    // so the second mark sits on the first.
    ("dejavu", "latin-mark-on-mark"),
    // Thai: SARA AM decomposition with the nikhahit moved before a
    // tone mark, and the selection of the legacy private-use glyph
    // variants (lowered / shifted marks, descender-less consonants)
    // in fonts without Thai `GSUB` — neither is specified in the
    // staged documentation.
    ("tahoma", "thai-below-vowels"),
    ("tahoma", "thai-sara-am"),
    ("arialunicode", "thai-tone-marks"),
    ("arialunicode", "thai-below-vowels"),
    ("arialunicode", "thai-sara-am"),
    // A font written for the older Indic shaping model (script tag
    // `deva` only): its matra and reph placement conventions are not
    // described in the staged specification beyond "all features are
    // applied to Consonant + Halant sequences".
    ("arialunicode", "deva-pre-base-matra"),
    ("arialunicode", "deva-reph"),
    ("arialunicode", "deva-marks"),
];

#[test]
fn shaping_matches_expected_runs() {
    let report = std::env::var_os("SCRIBE_SHAPING_REPORT").is_some();
    let regen = std::env::var_os("SCRIBE_SHAPING_REGEN").is_some();
    let live = reference_tool();
    let expected = load_expected();
    let mut regenerated = String::from(
        "# font\tcase\tcode points\texpected run (provenance: hb-shape --no-glyph-names, font units)\n",
    );

    let mut failures: Vec<String> = Vec::new();
    let mut stale_known: Vec<String> = Vec::new();
    let mut per_group: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut compared = 0usize;

    for font in FONTS {
        let Some(path) = resolve(font) else {
            if report {
                eprintln!("skip font {}: not installed", font.key);
            }
            continue;
        };
        if lacks_opentype_layout(&path, font.index) {
            if report {
                eprintln!(
                    "skip font {}: shaped by `morx` tables, which are not OpenType Layout",
                    font.key
                );
            }
            continue;
        }
        let Some(face) = load_face(&path, font.index) else {
            failures.push(format!("{}: font failed to load", font.key));
            continue;
        };
        let chain = FaceChain::new(face);
        let space = chain
            .shape(" ", 16.0)
            .ok()
            .and_then(|g| g.first().map(|g| u32::from(g.glyph_id)))
            .unwrap_or(u32::MAX);
        for case in CASES.iter().filter(|c| c.fonts.contains(&font.key)) {
            let live_line = if live {
                reference_shape(&path, font.index, case)
            } else {
                None
            };
            let stored = expected
                .get(&(font.key.to_string(), case.name.to_string()))
                .filter(|(cps, _)| *cps == unicodes(case.text))
                .map(|(_, line)| line.clone());
            if font.fixture
                && let Some(l) = &live_line
            {
                regenerated.push_str(&format!(
                    "{}\t{}\t{}\t{}\n",
                    font.key,
                    case.name,
                    unicodes(case.text),
                    l
                ));
            }
            let want_line = if regen {
                live_line.clone().or(stored)
            } else if font.fixture {
                // The committed expectation is authoritative for the
                // committed fonts.
                if stored.is_none() && !live {
                    failures.push(format!(
                        "{}/{}: no committed expectation (regenerate expected.tsv)",
                        font.key, case.name
                    ));
                    continue;
                }
                stored.or(live_line.clone())
            } else {
                live_line.clone()
            };
            let Some(want_line) = want_line else {
                continue;
            };
            let Some(want) = parse_reference(&want_line) else {
                failures.push(format!(
                    "{}/{}: unparsable expectation {want_line:?}",
                    font.key, case.name
                ));
                continue;
            };
            compared += 1;
            let group = case.name.split('-').next().unwrap_or("").to_string();
            let entry = per_group.entry(group).or_insert((0, 0));
            entry.1 += 1;
            let varied;
            let shaper = if case.variations.is_empty() {
                &chain
            } else {
                let mut face = load_face(&path, font.index).expect("loaded once already");
                let coords: Vec<f32> = face
                    .variation_axes()
                    .iter()
                    .map(|axis| {
                        case.variations
                            .iter()
                            .find(|(t, _)| t.as_bytes() == axis.tag)
                            .map_or(axis.default, |&(_, v)| v)
                    })
                    .collect();
                face.set_variation_coords(&coords)
                    .expect("variation coords");
                varied = FaceChain::new(face);
                &varied
            };
            let verdict = match scribe_shape(shaper, case) {
                Ok(got) => difference(&got, &want, space),
                Err(e) => Some(e),
            };
            let known = KNOWN_DEVIATIONS.contains(&(font.key, case.name));
            match verdict {
                None => {
                    entry.0 += 1;
                    if known {
                        stale_known.push(format!("{}/{}", font.key, case.name));
                    }
                    if report {
                        eprintln!("ok    {}/{}", font.key, case.name);
                    }
                }
                Some(why) => {
                    if report {
                        eprintln!(
                            "{} {}/{}: {why}",
                            if known { "known" } else { "FAIL " },
                            font.key,
                            case.name
                        );
                    }
                    if !known {
                        failures.push(format!("{}/{}: {why}", font.key, case.name));
                    }
                }
            }
        }
    }

    if regen && live {
        std::fs::create_dir_all(expected_path().parent().unwrap()).unwrap();
        std::fs::write(expected_path(), regenerated).unwrap();
    }
    if report {
        for (group, (ok, total)) in &per_group {
            eprintln!("{group:10} {ok}/{total}");
        }
        eprintln!("compared {compared} runs, {} mismatches", failures.len());
    }
    assert!(
        compared > 0,
        "no case ran: the committed fixtures are missing"
    );
    assert!(
        stale_known.is_empty(),
        "cases listed as known deviations now agree; remove them: {stale_known:?}"
    );
    assert!(
        failures.is_empty(),
        "{} shaping mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn reference_line_parser() {
    let run = parse_reference("[5=0+100|7=1@-20,30+0|9=2+50,-10]").unwrap();
    assert_eq!(run.glyphs.len(), 3);
    assert_eq!(
        (run.glyphs[1].gid, run.glyphs[1].x, run.glyphs[1].y),
        (7, 80.0, 30.0)
    );
    assert!(run.glyphs[1].stacked && !run.glyphs[2].stacked);
    assert_eq!(
        (run.glyphs[2].gid, run.glyphs[2].x, run.glyphs[2].y),
        (9, 100.0, 0.0)
    );
    assert_eq!(run.advance, (150.0, -10.0));
    assert_eq!(parse_reference("[]").unwrap().glyphs.len(), 0);
    assert!(parse_reference("nonsense").is_none());
}
