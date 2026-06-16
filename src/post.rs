//! `post` (PostScript) table — glyph-name resolution.
//!
//! The `post` table maps each glyph ID to a PostScript glyph name. Its
//! `version` field (a `Fixed`) selects a layout:
//!
//! - **1.0** (`0x00010000`) — the font's glyph set is *exactly* the 258
//!   standard Macintosh glyphs in canonical order. No per-glyph data is
//!   stored; glyph `gid` is named by [`STANDARD_MAC_GLYPH_NAMES`]`[gid]`.
//! - **2.0** (`0x00020000`) — the general case. `numGlyphs` followed by a
//!   `glyphNameIndex[numGlyphs]`: an index `< 258` selects the standard
//!   name at that position; an index `>= 258` selects the
//!   `(index - 258)`-th custom Pascal string from the table's own
//!   `stringData` array.
//! - **2.5** (`0x00028000`) — deprecated. `numGlyphs` followed by one
//!   *signed* byte per glyph: the glyph's standard index is
//!   `gid + offset[gid]`, then named via the standard table. Recognised
//!   for reading; not produced for OpenType.
//! - **3.0** (`0x00030000`) — supplies no names at all.
//!
//! All names resolve through the 258-entry standard Macintosh ordering.
//!
//! Spec references:
//! - Apple *TrueType Reference Manual*, `post` table chapter (the
//!   canonical 258-name list under "`post` Format 1").
//! - Microsoft OpenType §"post — PostScript Table" (format semantics;
//!   defers to Apple for the name list).
//!
//! Staged docs read for this module:
//! `docs/text/opentype/post-standard-mac-glyph-names.md`,
//! `docs/text/opentype/apple-chap6post.html`,
//! `docs/text/opentype/otspec-post.html`.

/// The 258 standard Macintosh glyph names, in canonical ordering. Index
/// `i` is the name for standard glyph index `i` (also glyph ID `i` in a
/// `post` format-1.0 font). All 258 entries are distinct.
pub static STANDARD_MAC_GLYPH_NAMES: [&str; 258] = [
    ".notdef",
    ".null",
    "nonmarkingreturn",
    "space",
    "exclam",
    "quotedbl",
    "numbersign",
    "dollar",
    "percent",
    "ampersand",
    "quotesingle",
    "parenleft",
    "parenright",
    "asterisk",
    "plus",
    "comma",
    "hyphen",
    "period",
    "slash",
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "colon",
    "semicolon",
    "less",
    "equal",
    "greater",
    "question",
    "at",
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "bracketleft",
    "backslash",
    "bracketright",
    "asciicircum",
    "underscore",
    "grave",
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
    "h",
    "i",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "q",
    "r",
    "s",
    "t",
    "u",
    "v",
    "w",
    "x",
    "y",
    "z",
    "braceleft",
    "bar",
    "braceright",
    "asciitilde",
    "Adieresis",
    "Aring",
    "Ccedilla",
    "Eacute",
    "Ntilde",
    "Odieresis",
    "Udieresis",
    "aacute",
    "agrave",
    "acircumflex",
    "adieresis",
    "atilde",
    "aring",
    "ccedilla",
    "eacute",
    "egrave",
    "ecircumflex",
    "edieresis",
    "iacute",
    "igrave",
    "icircumflex",
    "idieresis",
    "ntilde",
    "oacute",
    "ograve",
    "ocircumflex",
    "odieresis",
    "otilde",
    "uacute",
    "ugrave",
    "ucircumflex",
    "udieresis",
    "dagger",
    "degree",
    "cent",
    "sterling",
    "section",
    "bullet",
    "paragraph",
    "germandbls",
    "registered",
    "copyright",
    "trademark",
    "acute",
    "dieresis",
    "notequal",
    "AE",
    "Oslash",
    "infinity",
    "plusminus",
    "lessequal",
    "greaterequal",
    "yen",
    "mu",
    "partialdiff",
    "summation",
    "product",
    "pi",
    "integral",
    "ordfeminine",
    "ordmasculine",
    "Omega",
    "ae",
    "oslash",
    "questiondown",
    "exclamdown",
    "logicalnot",
    "radical",
    "florin",
    "approxequal",
    "Delta",
    "guillemotleft",
    "guillemotright",
    "ellipsis",
    "nonbreakingspace",
    "Agrave",
    "Atilde",
    "Otilde",
    "OE",
    "oe",
    "endash",
    "emdash",
    "quotedblleft",
    "quotedblright",
    "quoteleft",
    "quoteright",
    "divide",
    "lozenge",
    "ydieresis",
    "Ydieresis",
    "fraction",
    "currency",
    "guilsinglleft",
    "guilsinglright",
    "fi",
    "fl",
    "daggerdbl",
    "periodcentered",
    "quotesinglbase",
    "quotedblbase",
    "perthousand",
    "Acircumflex",
    "Ecircumflex",
    "Aacute",
    "Edieresis",
    "Egrave",
    "Iacute",
    "Icircumflex",
    "Idieresis",
    "Igrave",
    "Oacute",
    "Ocircumflex",
    "apple",
    "Ograve",
    "Uacute",
    "Ucircumflex",
    "Ugrave",
    "dotlessi",
    "circumflex",
    "tilde",
    "macron",
    "breve",
    "dotaccent",
    "ring",
    "cedilla",
    "hungarumlaut",
    "ogonek",
    "caron",
    "Lslash",
    "lslash",
    "Scaron",
    "scaron",
    "Zcaron",
    "zcaron",
    "brokenbar",
    "Eth",
    "eth",
    "Yacute",
    "yacute",
    "Thorn",
    "thorn",
    "minus",
    "multiply",
    "onesuperior",
    "twosuperior",
    "threesuperior",
    "onehalf",
    "onequarter",
    "threequarters",
    "franc",
    "Gbreve",
    "gbreve",
    "Idotaccent",
    "Scedilla",
    "scedilla",
    "Cacute",
    "cacute",
    "Ccaron",
    "ccaron",
    "dcroat",
];

/// Return the standard Macintosh glyph name for a standard index, or
/// `None` if `idx >= 258`.
#[inline]
pub fn standard_mac_glyph_name(idx: u16) -> Option<&'static str> {
    STANDARD_MAC_GLYPH_NAMES.get(idx as usize).copied()
}

/// A parsed `post` table, retaining enough state to resolve a glyph ID
/// to its PostScript name.
#[derive(Debug, Clone)]
pub struct PostTable {
    inner: PostKind,
}

#[derive(Debug, Clone)]
enum PostKind {
    /// Format 1.0 — every glyph named by the standard ordering. Holds
    /// the glyph count so out-of-range queries return `None`.
    Standard { num_glyphs: u16 },
    /// Format 2.0 — per-glyph index into either the standard ordering
    /// (`< 258`) or `custom` (`>= 258`, minus 258).
    Custom {
        glyph_name_index: Vec<u16>,
        custom: Vec<String>,
    },
    /// Format 2.5 — per-glyph signed delta to the standard index.
    Delta { offsets: Vec<i8> },
    /// Format 3.0 — no names available.
    NoNames,
}

impl PostTable {
    /// Parse a `post` table from its raw bytes. Returns `None` if the
    /// buffer is too short for the header or its declared layout, or if
    /// the version is unrecognised.
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 32 {
            return None;
        }
        // Header: version(Fixed) + a fixed block of metrics we don't use
        // (italicAngle, underlinePosition, underlineThickness, isFixedPitch,
        // 4 × min/max memory) totalling 32 bytes before any per-glyph data.
        let version = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        let inner = match version {
            0x0001_0000 => PostKind::Standard {
                num_glyphs: 258, // implied by the format
            },
            0x0002_0000 => Self::parse_format2(&b[32..])?,
            0x0002_8000 => Self::parse_format25(&b[32..])?,
            0x0003_0000 => PostKind::NoNames,
            // 2.5 in some fonts uses a slightly different encoding of the
            // version Fixed; only the canonical value is recognised. Any
            // unknown version is treated as "no names" rather than a hard
            // failure so a font with an exotic post still parses.
            _ => PostKind::NoNames,
        };
        Some(PostTable { inner })
    }

    /// Format 2.0 body (everything after the 32-byte header).
    fn parse_format2(body: &[u8]) -> Option<PostKind> {
        if body.len() < 2 {
            return None;
        }
        let num_glyphs = u16::from_be_bytes([body[0], body[1]]) as usize;
        let idx_end = 2 + num_glyphs * 2;
        if body.len() < idx_end {
            return None;
        }
        let mut glyph_name_index = Vec::with_capacity(num_glyphs);
        let mut max_custom: i64 = -1;
        for i in 0..num_glyphs {
            let o = 2 + i * 2;
            let v = u16::from_be_bytes([body[o], body[o + 1]]);
            if v >= 258 {
                max_custom = max_custom.max((v - 258) as i64);
            }
            glyph_name_index.push(v);
        }
        // Parse the Pascal-string array that follows the index. We read
        // sequentially; a string referenced past the end of stringData
        // resolves to None at lookup time.
        let mut custom: Vec<String> = Vec::new();
        let mut cur = idx_end;
        // Read until we run out of bytes or have collected every custom
        // string the index actually references.
        while cur < body.len() && (custom.len() as i64) <= max_custom {
            let len = body[cur] as usize;
            cur += 1;
            if cur + len > body.len() {
                // Truncated final string — keep what's valid, then stop.
                break;
            }
            // Names are ASCII per the spec; preserve bytes losslessly.
            custom.push(String::from_utf8_lossy(&body[cur..cur + len]).into_owned());
            cur += len;
        }
        Some(PostKind::Custom {
            glyph_name_index,
            custom,
        })
    }

    /// Format 2.5 body (everything after the 32-byte header).
    fn parse_format25(body: &[u8]) -> Option<PostKind> {
        if body.len() < 2 {
            return None;
        }
        let num_glyphs = u16::from_be_bytes([body[0], body[1]]) as usize;
        if body.len() < 2 + num_glyphs {
            return None;
        }
        let offsets = body[2..2 + num_glyphs].iter().map(|&x| x as i8).collect();
        Some(PostKind::Delta { offsets })
    }

    /// Resolve glyph ID `gid` to its PostScript name. Returns `None`
    /// when the table supplies no name for that glyph (format 3.0, an
    /// out-of-range glyph, a format-2.0 index `0` mapping to `.notdef`
    /// is still returned, but a dangling custom index returns `None`,
    /// and a format-2.5 delta resolving outside the standard set
    /// returns `None`).
    pub fn glyph_name(&self, gid: u16) -> Option<&str> {
        match &self.inner {
            PostKind::Standard { num_glyphs } => {
                if gid < *num_glyphs {
                    standard_mac_glyph_name(gid)
                } else {
                    None
                }
            }
            PostKind::Custom {
                glyph_name_index,
                custom,
            } => {
                let v = *glyph_name_index.get(gid as usize)?;
                if v < 258 {
                    standard_mac_glyph_name(v)
                } else {
                    custom.get((v - 258) as usize).map(|s| s.as_str())
                }
            }
            PostKind::Delta { offsets } => {
                let off = *offsets.get(gid as usize)?;
                // standard_index = gid + offset[gid]; must land in 0..258.
                let std = i32::from(gid) + i32::from(off);
                if (0..258).contains(&std) {
                    standard_mac_glyph_name(std as u16)
                } else {
                    None
                }
            }
            PostKind::NoNames => None,
        }
    }

    /// `true` when this table can resolve at least some glyph names
    /// (any format except 3.0).
    pub fn has_names(&self) -> bool {
        !matches!(self.inner, PostKind::NoNames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_table_is_258_distinct_names() {
        assert_eq!(STANDARD_MAC_GLYPH_NAMES.len(), 258);
        // Anchors from the spec table.
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[0], ".notdef");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[1], ".null");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[2], "nonmarkingreturn");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[3], "space");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[36], "A");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[68], "a");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[192], "fi");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[193], "fl");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[210], "apple");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[257], "dcroat");

        // All names distinct.
        let mut seen = std::collections::HashSet::new();
        for n in STANDARD_MAC_GLYPH_NAMES.iter() {
            assert!(seen.insert(*n), "duplicate name {n}");
        }
        assert_eq!(seen.len(), 258);
    }

    #[test]
    fn standard_index_accessor_bounds() {
        assert_eq!(standard_mac_glyph_name(0), Some(".notdef"));
        assert_eq!(standard_mac_glyph_name(257), Some("dcroat"));
        assert_eq!(standard_mac_glyph_name(258), None);
        assert_eq!(standard_mac_glyph_name(u16::MAX), None);
    }

    /// Build a minimal 32-byte post header with the given version.
    fn header(version: u32) -> Vec<u8> {
        let mut v = version.to_be_bytes().to_vec();
        v.extend(std::iter::repeat(0u8).take(28)); // pad to 32 bytes
        v
    }

    #[test]
    fn format_1_0_uses_standard_ordering() {
        let t = PostTable::parse(&header(0x0001_0000)).unwrap();
        assert!(t.has_names());
        assert_eq!(t.glyph_name(0), Some(".notdef"));
        assert_eq!(t.glyph_name(36), Some("A"));
        assert_eq!(t.glyph_name(257), Some("dcroat"));
        // Beyond the standard 258 -> no name.
        assert_eq!(t.glyph_name(258), None);
        assert_eq!(t.glyph_name(1000), None);
    }

    #[test]
    fn format_3_0_has_no_names() {
        let t = PostTable::parse(&header(0x0003_0000)).unwrap();
        assert!(!t.has_names());
        assert_eq!(t.glyph_name(0), None);
        assert_eq!(t.glyph_name(36), None);
    }

    #[test]
    fn format_2_0_standard_and_custom() {
        // 3 glyphs:
        //   gid0 -> index 0   (.notdef, standard)
        //   gid1 -> index 258 (custom string #0)
        //   gid2 -> index 259 (custom string #1)
        let mut b = header(0x0002_0000);
        b.extend_from_slice(&3u16.to_be_bytes()); // numGlyphs
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&258u16.to_be_bytes());
        b.extend_from_slice(&259u16.to_be_bytes());
        // stringData: two Pascal strings.
        b.push(5);
        b.extend_from_slice(b"hello");
        b.push(3);
        b.extend_from_slice(b"foo");

        let t = PostTable::parse(&b).unwrap();
        assert!(t.has_names());
        assert_eq!(t.glyph_name(0), Some(".notdef"));
        assert_eq!(t.glyph_name(1), Some("hello"));
        assert_eq!(t.glyph_name(2), Some("foo"));
        // No glyph 3.
        assert_eq!(t.glyph_name(3), None);
    }

    #[test]
    fn format_2_0_dangling_custom_index_is_none() {
        // gid0 references custom string #0 but none are stored.
        let mut b = header(0x0002_0000);
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&258u16.to_be_bytes());
        // No stringData.
        let t = PostTable::parse(&b).unwrap();
        assert_eq!(t.glyph_name(0), None);
    }

    #[test]
    fn format_2_5_signed_delta() {
        // gid1,2,3 placed at standard indices 37,38,39 via delta +36
        // (Apple's worked example). gid0 -> +0 -> .notdef.
        let mut b = header(0x0002_8000);
        b.extend_from_slice(&4u16.to_be_bytes()); // numGlyphs
        b.push(0i8 as u8); // gid0 -> 0
        b.push(36i8 as u8); // gid1 -> 37
        b.push(36i8 as u8); // gid2 -> 38
        b.push(36i8 as u8); // gid3 -> 39
        let t = PostTable::parse(&b).unwrap();
        assert_eq!(t.glyph_name(0), Some(".notdef"));
        assert_eq!(t.glyph_name(1), Some("B")); // index 37
        assert_eq!(t.glyph_name(2), Some("C")); // index 38
        assert_eq!(t.glyph_name(3), Some("D")); // index 39
    }

    #[test]
    fn format_2_5_negative_delta_and_out_of_range() {
        let mut b = header(0x0002_8000);
        b.extend_from_slice(&2u16.to_be_bytes());
        b.push((-1i8) as u8); // gid0 -> -1 -> out of range
        b.push(1i8 as u8); // gid1 -> 2 -> nonmarkingreturn
        let t = PostTable::parse(&b).unwrap();
        assert_eq!(t.glyph_name(0), None);
        assert_eq!(t.glyph_name(1), Some("nonmarkingreturn"));
    }

    #[test]
    fn short_buffer_returns_none() {
        assert!(PostTable::parse(&[0, 1, 0]).is_none());
        // Exactly-32-byte format 2.0 with no numGlyphs body -> None.
        assert!(PostTable::parse(&header(0x0002_0000)).is_none());
    }

    #[test]
    fn unknown_version_parses_as_no_names() {
        let t = PostTable::parse(&header(0x0004_0000)).unwrap();
        assert!(!t.has_names());
        assert_eq!(t.glyph_name(0), None);
    }
}
