//! Byte-exact text encoding handling: detect, decode, encode.
//!
//! This module is the heart of the do-no-harm rule (AGENTS.md, features §4.5):
//! loading then saving a foreign file must be BYTE-IDENTICAL — preserve the
//! encoding, the BOM, the line endings and the trailing newline. The theorem
//! this module maintains: decode(encode(text, d), d) == text, and
//! encode(decode(bytes, detect(bytes, cp)), detect(bytes, cp)) == bytes — or
//! a typed error; never a silent rewrite, never a panic on hostile bytes.
//!
//! Pure std only: no windows crate, no OS calls. The one OS-specific fact
//! core needs — the active ANSI code page — arrives as a PARAMETER
//! (default_ansi_codepage); api/bridge supplies GetACP() later. Core decides
//! nothing about the OS.
//!
//! Detection order (exact — reordering turns a BOM-less UTF-16 file into
//! mojibake):
//! 1. BOM: FF FE -> UTF-16LE, FE FF -> UTF-16BE, EF BB BF -> UTF-8, each with
//!    bom_present = true;
//! 2. NUL-parity heuristic for BOM-less UTF-16: even byte length and every
//!    NUL at an odd offset -> UTF-16LE; every NUL at an even offset ->
//!    UTF-16BE. Requires at least one NUL. Trade-off, deliberately chosen and
//!    written down: a UTF-16 file containing NO NULs (impossible for ASCII
//!    text, possible for a one-character CJK file) is indistinguishable from
//!    ANSI/UTF-8, and we deliberately read it as UTF-8/ANSI instead of
//!    guessing UTF-16 — misreading such a file produces different characters
//!    but never different bytes on write-back;
//! 3. valid UTF-8 -> Utf8;
//! 4. otherwise the caller's ANSI code page is reported (Ansi(cp)) and decode
//!    — not detect — refuses anything we cannot map losslessly: detect never
//!    guesses a code page that would silently corrupt a file, and an
//!    unsupported code page surfaces as UnsupportedCodepage so the caller
//!    opens the file read-only. With default_ansi_codepage = None there is no
//!    code page to try at all: the encoding still reports Utf8 (so decode can
//!    name the exact malformed byte) and decode errors MalformedUtf8 —
//!    guessing 1252 for an unknown-code-page file could silently mis-decode
//!    it, which is precisely the harm this module exists to prevent.
//!
//! Line endings: CRLF is dominant only when CRLFs exist and no lone LF does;
//! lone CRs (classic Mac) count as Lf for the write-back decision but are
//! preserved byte-exactly by decode/encode like every other byte.
//!
//! Size guard (D9): MAX_TEXT_BYTES / is_oversize — at or above the limit the
//! CALLER opens the file read-only; decode/encode never refuse on size, they
//! are pure transforms.

/// The documented size guard (D9): files at or above this many bytes are
/// opened read-only by the caller. Never a refusal inside this module.
pub const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;

/// The ANSI code pages this module can map losslessly. Windows-1252 only,
/// on purpose: it is the overwhelmingly common Windows ANSI code page, and
/// every additional page is another chance to silently corrupt someone's
/// file — the one failure mode the do-no-harm rule exists to prevent. Any
/// other code page decodes as UnsupportedCodepage and the caller opens the
/// file read-only, preserving the bytes exactly.
pub const SUPPORTED_ANSI_CODEPAGES: &[u16] = &[1252];

/// True when a file of this size must be opened read-only (D9: at or above
/// MAX_TEXT_BYTES — never refuse, never truncate).
pub fn is_oversize(len_bytes: usize) -> bool {
    len_bytes >= MAX_TEXT_BYTES
}

/// A text encoding, including the ANSI code page when relevant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    /// UTF-8 without a BOM.
    Utf8,
    /// UTF-8 with a BOM (the BOM is part of the file, not of the text).
    Utf8Bom,
    /// UTF-16, little-endian.
    Utf16Le,
    /// UTF-16, big-endian.
    Utf16Be,
    /// A Windows ANSI code page; only 1252 is supported (see
    /// SUPPORTED_ANSI_CODEPAGES).
    Ansi(u16),
}

/// Dominant line ending of a file. Lone CRs are NOT represented — they count
/// as Lf for the write-back decision and are preserved byte-exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

/// What detect() concluded about a byte sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detected {
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    /// The file ends with a newline (for UTF-16: a final U+000A unit; for
    /// CRLF files a final CRLF counts as true).
    pub trailing_newline: bool,
    pub bom_present: bool,
}

/// Decode failures. Typed and total: a hostile file produces an error with
/// the offending byte offset, never a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("malformed utf-8 at byte {byte_offset}")]
    MalformedUtf8 {
        /// Offset of the first byte that is not part of a valid sequence.
        byte_offset: usize,
    },
    #[error("unsupported ansi codepage {0}")]
    UnsupportedCodepage(u16),
    #[error("unterminated utf-16 sequence at byte {byte_offset}")]
    /// A UTF-16 code-unit sequence that ends mid-unit, or an unpaired
    /// surrogate — a high surrogate with no low one after it, or a low
    /// surrogate with no high one before it, wherever it occurs in the
    /// input (not only at its end). byte_offset names the first byte of
    /// the offending code unit.
    UnterminatedUtf16 {
        /// Offset of the first byte of the offending UTF-16 code unit (an
        /// unpaired surrogate, or the truncated final unit).
        byte_offset: usize,
    },
}

/// Encode failures. Mirrors api::SaveError::Unencodable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    #[error("character {code_point:?} is not representable in codepage {codepage}")]
    Unencodable {
        /// The exact character that has no byte in the target code page.
        /// Never silently written as '?' or a box glyph.
        code_point: char,
        codepage: u16,
    },
}

/// Detects encoding, dominant line ending and trailing newline.
pub fn detect(bytes: &[u8], default_ansi_codepage: Option<u16>) -> Detected {
    let (encoding, bom_present) = detect_encoding(bytes, default_ansi_codepage);
    Detected {
        encoding,
        bom_present,
        line_ending: detect_line_ending(bytes, encoding),
        trailing_newline: has_trailing_newline(bytes, encoding),
    }
}

/// Decodes bytes to text per the detection result. The BOM is NOT part of
/// the returned text; encode() re-emits it because bom_present is carried in
/// Detected.
pub fn decode(bytes: &[u8], d: Detected) -> Result<String, DecodeError> {
    match d.encoding {
        TextEncoding::Utf8 => utf8_str(bytes),
        TextEncoding::Utf8Bom => utf8_str(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)),
        TextEncoding::Utf16Le => utf16_str(bytes, false, d.bom_present),
        TextEncoding::Utf16Be => utf16_str(bytes, true, d.bom_present),
        TextEncoding::Ansi(1252) => cp1252_str(bytes),
        TextEncoding::Ansi(other) => Err(DecodeError::UnsupportedCodepage(other)),
    }
}

/// Encodes text per the detection result. A character with no byte in the
/// target code page is refused exactly (Unencodable) — never written as
/// '?' or a box glyph.
pub fn encode(text: &str, d: Detected) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::with_capacity(text.len() + 4);
    match d.encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => {
            if d.bom_present {
                out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            }
            out.extend_from_slice(text.as_bytes());
            Ok(out)
        }
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            let be = d.encoding == TextEncoding::Utf16Be;
            if d.bom_present {
                push_unit(&mut out, 0xFEFF, be);
            }
            for unit in text.encode_utf16() {
                push_unit(&mut out, unit, be);
            }
            Ok(out)
        }
        TextEncoding::Ansi(1252) => {
            for ch in text.chars() {
                match u32::from(ch) {
                    // ASCII and the Latin-1 range are identity bytes in 1252.
                    c @ 0x00..=0x7F => out.push(c as u8),
                    c @ 0xA0..=0xFF => out.push(c as u8),
                    _ => match cp1252_char_to_byte(ch) {
                        Some(b) => out.push(b),
                        None => {
                            return Err(EncodeError::Unencodable {
                                code_point: ch,
                                codepage: 1252,
                            });
                        }
                    },
                }
            }
            Ok(out)
        }
        TextEncoding::Ansi(other) => match text.chars().next() {
            // Nothing to encode -> an empty output is trivially correct.
            None => Ok(out),
            // We hold no lossless table for this code page, so the first
            // character is already unencodable; refusing exactly is the
            // do-no-harm behaviour.
            Some(ch) => Err(EncodeError::Unencodable {
                code_point: ch,
                codepage: other,
            }),
        },
    }
}

/// The whole point: decode(encode(text, d), d) == text AND
/// encode(decode(bytes, detect(bytes, cp)), detect(bytes, cp)) == bytes.
/// On success the output is byte-identical to the input; on failure a typed
/// DecodeError (the caller opens the file read-only and the bytes survive).
pub fn round_trip(
    bytes: &[u8],
    default_ansi_codepage: Option<u16>,
) -> Result<Vec<u8>, DecodeError> {
    let d = detect(bytes, default_ansi_codepage);
    let text = decode(bytes, d)?;
    // encode() cannot fail for a Detected produced by detect(): every char
    // decode() produced came from a byte encode() can express. The mapping
    // keeps the function total anyway — typed error, never a panic.
    encode(&text, d).map_err(|e| match e {
        EncodeError::Unencodable { codepage, .. } => DecodeError::UnsupportedCodepage(codepage),
    })
}

// --- detection internals ---

fn detect_encoding(bytes: &[u8], default_ansi_codepage: Option<u16>) -> (TextEncoding, bool) {
    // 1. BOMs decide.
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return (TextEncoding::Utf16Le, true);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return (TextEncoding::Utf16Be, true);
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (TextEncoding::Utf8Bom, true);
    }
    // 2. NUL-parity heuristic for BOM-less UTF-16 (needs at least one NUL;
    //    a file with no NULs is deliberately read as UTF-8/ANSI — see the
    //    module docs for the trade-off).
    if let Some(enc) = bom_less_utf16(bytes) {
        return (enc, false);
    }
    // 3. Valid UTF-8.
    if std::str::from_utf8(bytes).is_ok() {
        return (TextEncoding::Utf8, false);
    }
    // 4. ANSI — reported, never guessed into: decode refuses what we cannot
    //    map losslessly.
    match default_ansi_codepage {
        Some(cp) => (TextEncoding::Ansi(cp), false),
        // No code page supplied: report Utf8 so decode can name the exact
        // malformed byte instead of guessing 1252 and silently mis-decoding
        // a file that is really some other code page.
        None => (TextEncoding::Utf8, false),
    }
}

/// BOM-less UTF-16 by NUL parity: even byte length and all NULs on one side.
/// None means "not UTF-16" — including the ambiguous no-NULs case.
fn bom_less_utf16(bytes: &[u8]) -> Option<TextEncoding> {
    if bytes.len() < 2 || bytes.len() % 2 != 0 {
        return None;
    }
    let mut nul_at_odd = false;
    let mut nul_at_even = false;
    for (i, &b) in bytes.iter().enumerate() {
        if b == 0 {
            if i % 2 == 0 {
                nul_at_even = true;
            } else {
                nul_at_odd = true;
            }
        }
    }
    if nul_at_odd && !nul_at_even {
        Some(TextEncoding::Utf16Le)
    } else if nul_at_even && !nul_at_odd {
        Some(TextEncoding::Utf16Be)
    } else {
        None
    }
}

fn detect_line_ending(bytes: &[u8], encoding: TextEncoding) -> LineEnding {
    let (mut has_crlf, mut has_lone_lf) = (false, false);
    match encoding {
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            let be = encoding == TextEncoding::Utf16Be;
            let units = bytes.len() / 2;
            let unit = |i: usize| -> u16 {
                if be {
                    u16::from_be_bytes([bytes[2 * i], bytes[2 * i + 1]])
                } else {
                    u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]])
                }
            };
            let mut i = 0;
            while i < units {
                if unit(i) == 0x0D && i + 1 < units && unit(i + 1) == 0x0A {
                    has_crlf = true;
                    i += 2;
                } else {
                    if unit(i) == 0x0A {
                        has_lone_lf = true;
                    }
                    i += 1;
                }
            }
        }
        _ => {
            let mut i = 0;
            while i < bytes.len() {
                match bytes[i] {
                    b'\r' if i + 1 < bytes.len() && bytes[i + 1] == b'\n' => {
                        has_crlf = true;
                        i += 2;
                    }
                    b'\n' => {
                        has_lone_lf = true;
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
        }
    }
    if has_crlf && !has_lone_lf {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}

fn has_trailing_newline(bytes: &[u8], encoding: TextEncoding) -> bool {
    match encoding {
        // A UTF-16 newline is a two-byte unit; the low byte would make every
        // such file "no trailing newline" if we looked at the last byte only.
        TextEncoding::Utf16Le => bytes.ends_with(&[0x0A, 0x00]),
        TextEncoding::Utf16Be => bytes.ends_with(&[0x00, 0x0A]),
        _ => bytes.ends_with(b"\n"),
    }
}

// --- decode internals ---

fn utf8_str(bytes: &[u8]) -> Result<String, DecodeError> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|e| DecodeError::MalformedUtf8 {
            byte_offset: e.valid_up_to(),
        })
}

fn utf16_str(bytes: &[u8], be: bool, bom: bool) -> Result<String, DecodeError> {
    let body: &[u8] = if bom {
        let bom: [u8; 2] = if be { [0xFE, 0xFF] } else { [0xFF, 0xFE] };
        bytes.strip_prefix(&bom).unwrap_or(bytes)
    } else {
        bytes
    };
    if body.len() % 2 != 0 {
        return Err(DecodeError::UnterminatedUtf16 {
            byte_offset: body.len() - 1,
        });
    }
    let unit = |i: usize| -> u16 {
        if be {
            u16::from_be_bytes([body[2 * i], body[2 * i + 1]])
        } else {
            u16::from_le_bytes([body[2 * i], body[2 * i + 1]])
        }
    };
    let mut out = String::new();
    let mut i = 0;
    while i * 2 < body.len() {
        let u = unit(i);
        let ch = if (0xD800..=0xDBFF).contains(&u) {
            // High surrogate: must be paired with a low one immediately after.
            if (i + 1) * 2 < body.len() && (0xDC00..=0xDFFF).contains(&unit(i + 1)) {
                let next = unit(i + 1);
                let scalar = 0x10000 + (u32::from(u - 0xD800)) * 0x400 + u32::from(next - 0xDC00);
                i += 2;
                scalar
            } else {
                return Err(DecodeError::UnterminatedUtf16 { byte_offset: i * 2 });
            }
        } else if (0xDC00..=0xDFFF).contains(&u) {
            // Unpaired low surrogate first: refuse, never replace.
            return Err(DecodeError::UnterminatedUtf16 { byte_offset: i * 2 });
        } else {
            i += 1;
            u32::from(u)
        };
        // The ranges above are valid scalars by construction; the typed
        // fallback keeps this total instead of trusting that blindly.
        let Some(ch) = char::from_u32(ch) else {
            return Err(DecodeError::UnterminatedUtf16 { byte_offset: i * 2 });
        };
        out.push(ch);
    }
    Ok(out)
}

/// Windows-1252 for the 0x80..=0x9F range: the 27 slots that map to Unicode.
/// The five undefined slots (0x81, 0x8D, 0x8F, 0x90, 0x9D) return None —
/// they must never be silently mapped to a look-alike or a replacement.
fn cp1252_high_byte(b: u8) -> Option<char> {
    match b {
        0x80 => Some('\u{20AC}'), // EURO SIGN
        0x82 => Some('\u{201A}'), // SINGLE LOW-9 QUOTATION MARK
        0x83 => Some('\u{0192}'), // LATIN SMALL LETTER F WITH HOOK
        0x84 => Some('\u{201E}'), // DOUBLE LOW-9 QUOTATION MARK
        0x85 => Some('\u{2026}'), // HORIZONTAL ELLIPSIS
        0x86 => Some('\u{2020}'), // DAGGER
        0x87 => Some('\u{2021}'), // DOUBLE DAGGER
        0x88 => Some('\u{02C6}'), // MODIFIER LETTER CIRCUMFLEX ACCENT
        0x89 => Some('\u{2030}'), // PER MILLE SIGN
        0x8A => Some('\u{0160}'), // LATIN CAPITAL LETTER S WITH CARON
        0x8B => Some('\u{2039}'), // SINGLE LEFT-POINTING ANGLE QUOTATION MARK
        0x8C => Some('\u{0152}'), // LATIN CAPITAL LIGATURE OE
        0x8E => Some('\u{017D}'), // LATIN CAPITAL LETTER Z WITH CARON
        0x91 => Some('\u{2018}'), // LEFT SINGLE QUOTATION MARK
        0x92 => Some('\u{2019}'), // RIGHT SINGLE QUOTATION MARK
        0x93 => Some('\u{201C}'), // LEFT DOUBLE QUOTATION MARK
        0x94 => Some('\u{201D}'), // RIGHT DOUBLE QUOTATION MARK
        0x95 => Some('\u{2022}'), // BULLET
        0x96 => Some('\u{2013}'), // EN DASH
        0x97 => Some('\u{2014}'), // EM DASH
        0x98 => Some('\u{02DC}'), // SMALL TILDE
        0x99 => Some('\u{2122}'), // TRADE MARK SIGN
        0x9A => Some('\u{0161}'), // LATIN SMALL LETTER S WITH CARON
        0x9B => Some('\u{203A}'), // SINGLE RIGHT-POINTING ANGLE QUOTATION MARK
        0x9C => Some('\u{0153}'), // LATIN SMALL LIGATURE OE
        0x9E => Some('\u{017E}'), // LATIN SMALL LETTER Z WITH CARON
        0x9F => Some('\u{0178}'), // LATIN CAPITAL LETTER Y WITH DIAERESIS
        _ => None,                // 0x81, 0x8D, 0x8F, 0x90, 0x9D: undefined
    }
}

fn cp1252_str(bytes: &[u8]) -> Result<String, DecodeError> {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        let ch = match b {
            0x80..=0x9F => match cp1252_high_byte(b) {
                Some(ch) => ch,
                // The five undefined slots: refuse rather than mojibake.
                None => return Err(DecodeError::UnsupportedCodepage(1252)),
            },
            // ASCII and 0xA0..=0xFF are identity bytes in 1252.
            _ => b as char,
        };
        out.push(ch);
    }
    Ok(out)
}

// --- encode internals ---

fn push_unit(out: &mut Vec<u8>, unit: u16, be: bool) {
    let [lo, hi] = unit.to_le_bytes();
    if be {
        out.push(hi);
        out.push(lo);
    } else {
        out.push(lo);
        out.push(hi);
    }
}

/// The inverse of cp1252_high_byte: the 27 Unicode characters that have a
/// home in the 0x80..=0x9F range.
fn cp1252_char_to_byte(ch: char) -> Option<u8> {
    match ch {
        '\u{20AC}' => Some(0x80),
        '\u{201A}' => Some(0x82),
        '\u{0192}' => Some(0x83),
        '\u{201E}' => Some(0x84),
        '\u{2026}' => Some(0x85),
        '\u{2020}' => Some(0x86),
        '\u{2021}' => Some(0x87),
        '\u{02C6}' => Some(0x88),
        '\u{2030}' => Some(0x89),
        '\u{0160}' => Some(0x8A),
        '\u{2039}' => Some(0x8B),
        '\u{0152}' => Some(0x8C),
        '\u{017D}' => Some(0x8E),
        '\u{2018}' => Some(0x91),
        '\u{2019}' => Some(0x92),
        '\u{201C}' => Some(0x93),
        '\u{201D}' => Some(0x94),
        '\u{2022}' => Some(0x95),
        '\u{2013}' => Some(0x96),
        '\u{2014}' => Some(0x97),
        '\u{02DC}' => Some(0x98),
        '\u{2122}' => Some(0x99),
        '\u{0161}' => Some(0x9A),
        '\u{203A}' => Some(0x9B),
        '\u{0153}' => Some(0x9C),
        '\u{017E}' => Some(0x9E),
        '\u{0178}' => Some(0x9F),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "Hi \u{20AC} \u{2014} \u{00E9}"; // "Hi € — é"

    fn det(encoding: TextEncoding, bom: bool) -> Detected {
        Detected {
            encoding,
            line_ending: LineEnding::Lf,
            trailing_newline: false,
            bom_present: bom,
        }
    }

    fn utf8_bytes() -> Vec<u8> {
        vec![
            0x48, 0x69, 0x20, // "Hi "
            0xE2, 0x82, 0xAC, // €
            0x20, //
            0xE2, 0x80, 0x94, // —
            0x20, //
            0xC3, 0xA9, // é
        ]
    }

    #[test]
    fn utf8_round_trip_is_byte_exact() -> Result<(), Box<dyn std::error::Error>> {
        let d = det(TextEncoding::Utf8, false);
        let bytes = encode(TEXT, d)?;
        assert_eq!(bytes, utf8_bytes());
        assert_eq!(decode(&bytes, d)?, TEXT);
        assert_eq!(round_trip(&bytes, None)?, bytes);
        Ok(())
    }

    #[test]
    fn utf8_bom_round_trip_is_byte_exact() -> Result<(), Box<dyn std::error::Error>> {
        let d = det(TextEncoding::Utf8Bom, true);
        let bytes = encode(TEXT, d)?;
        let mut expected = vec![0xEF, 0xBB, 0xBF];
        expected.extend(utf8_bytes());
        assert_eq!(bytes, expected);
        assert_eq!(decode(&bytes, d)?, TEXT, "BOM is not part of the text");
        assert_eq!(round_trip(&bytes, None)?, bytes);
        Ok(())
    }

    #[test]
    fn utf16_le_with_bom_round_trip_is_byte_exact() -> Result<(), Box<dyn std::error::Error>> {
        let d = det(TextEncoding::Utf16Le, true);
        let bytes = encode(TEXT, d)?;
        assert_eq!(
            bytes,
            vec![
                0xFF, 0xFE, // BOM
                0x48, 0x00, 0x69, 0x00, 0x20, 0x00, // H i space
                0xAC, 0x20, // €
                0x20, 0x00, //
                0x14, 0x20, // —
                0x20, 0x00, //
                0xE9, 0x00, // é
            ]
        );
        assert_eq!(decode(&bytes, d)?, TEXT);
        assert_eq!(round_trip(&bytes, None)?, bytes);
        Ok(())
    }

    #[test]
    fn utf16_be_with_bom_round_trip_is_byte_exact() -> Result<(), Box<dyn std::error::Error>> {
        let d = det(TextEncoding::Utf16Be, true);
        let bytes = encode(TEXT, d)?;
        assert_eq!(
            bytes,
            vec![
                0xFE, 0xFF, // BOM
                0x00, 0x48, 0x00, 0x69, 0x00, 0x20, // H i space
                0x20, 0xAC, // €
                0x00, 0x20, //
                0x20, 0x14, // —
                0x00, 0x20, //
                0x00, 0xE9, // é
            ]
        );
        assert_eq!(decode(&bytes, d)?, TEXT);
        assert_eq!(round_trip(&bytes, None)?, bytes);
        Ok(())
    }

    #[test]
    fn utf16_le_without_bom_detected_by_nul_heuristic() -> Result<(), Box<dyn std::error::Error>> {
        // Same text, no BOM: every NUL sits at an odd offset -> UTF-16LE,
        // bom_present false, and the round trip is still byte-exact.
        let bytes = encode(TEXT, det(TextEncoding::Utf16Le, false))?;
        assert_eq!(bytes[0], 0x48); // "H", no BOM prefix
        let d = detect(&bytes, None);
        assert_eq!(d.encoding, TextEncoding::Utf16Le);
        assert!(!d.bom_present);
        assert_eq!(round_trip(&bytes, None)?, bytes);
        Ok(())
    }

    #[test]
    fn ansi_1252_round_trip_is_byte_exact() -> Result<(), Box<dyn std::error::Error>> {
        let d = det(TextEncoding::Ansi(1252), false);
        let bytes = encode(TEXT, d)?;
        assert_eq!(
            bytes,
            vec![
                0x48, 0x69, 0x20, // "Hi "
                0x80, // €
                0x20, //
                0x97, // —
                0x20, //
                0xE9, // é
            ]
        );
        assert_eq!(decode(&bytes, d)?, TEXT);
        assert_eq!(round_trip(&bytes, Some(1252))?, bytes);
        Ok(())
    }

    #[test]
    fn malformed_utf8_reports_the_exact_byte_offset() {
        // Detected as Utf8 directly, and via detect() with no code page
        // supplied (None means: never guess 1252, refuse instead).
        let d = det(TextEncoding::Utf8, false);
        let Err(e) = decode(&[b'a', b'b', b'c', 0xFF, b'd'], d) else {
            panic!("invalid utf-8 must fail");
        };
        assert_eq!(e, DecodeError::MalformedUtf8 { byte_offset: 3 });
        let Err(e) = round_trip(&[b'a', b'b', b'c', 0xFF, b'd'], None) else {
            panic!("invalid utf-8 with no code page must fail");
        };
        assert_eq!(e, DecodeError::MalformedUtf8 { byte_offset: 3 });
    }

    #[test]
    fn lone_utf16_surrogates_report_the_byte_offset() {
        let d = det(TextEncoding::Utf16Le, false);
        // 'A' followed by an unpaired HIGH surrogate D800: error at byte 2.
        let Err(e) = decode(&[0x41, 0x00, 0x00, 0xD8], d) else {
            panic!("lone high surrogate must fail");
        };
        assert_eq!(e, DecodeError::UnterminatedUtf16 { byte_offset: 2 });
        // An unpaired LOW surrogate first: error at byte 0.
        let Err(e) = decode(&[0x00, 0xDC, 0x41, 0x00], d) else {
            panic!("unpaired low surrogate must fail");
        };
        assert_eq!(e, DecodeError::UnterminatedUtf16 { byte_offset: 0 });
        // A truncated final unit (odd byte count): error at its start.
        let Err(e) = decode(&[0x41, 0x00, 0xD8], d) else {
            panic!("truncated utf-16 unit must fail");
        };
        assert_eq!(e, DecodeError::UnterminatedUtf16 { byte_offset: 2 });
    }

    #[test]
    fn mixed_line_endings_survive_and_report_the_dominant() -> Result<(), Box<dyn std::error::Error>>
    {
        // LF then CRLF then a lone CR: Lf is the documented dominant value,
        // and the bytes — including the lone CR — survive exactly.
        let mixed: &[u8] = b"a\nb\r\nc\r";
        let d = detect(mixed, Some(1252));
        assert_eq!(d.encoding, TextEncoding::Utf8);
        assert_eq!(d.line_ending, LineEnding::Lf, "a lone LF beats CRLF");
        assert_eq!(round_trip(mixed, Some(1252))?, mixed);

        // Same text as BOM'd UTF-16LE: the unit-level scan sees the same mix.
        let utf16 = encode("a\nb\r\nc\r", det(TextEncoding::Utf16Le, true))?;
        let d = detect(&utf16, None);
        assert_eq!(d.encoding, TextEncoding::Utf16Le);
        assert_eq!(d.line_ending, LineEnding::Lf);
        assert_eq!(round_trip(&utf16, None)?, utf16);

        // Pure CRLF reports CrLf.
        let crlf: &[u8] = b"a\r\nb\r\n";
        assert_eq!(detect(crlf, Some(1252)).line_ending, LineEnding::CrLf);
        assert_eq!(round_trip(crlf, Some(1252))?, crlf);
        Ok(())
    }

    #[test]
    fn trailing_newline_matrix_is_byte_exact() -> Result<(), Box<dyn std::error::Error>> {
        let bodies = ["x", "x\n", "x\r\n"];
        let encodings = [
            det(TextEncoding::Utf8, false),
            det(TextEncoding::Utf8Bom, true),
            det(TextEncoding::Utf16Le, true),
        ];
        for body in bodies {
            for d in encodings {
                let bytes = encode(body, d)?;
                let want = body.ends_with('\n');
                let seen = detect(&bytes, Some(1252));
                assert_eq!(
                    seen.trailing_newline, want,
                    "trailing newline wrong for {body:?} as {:?}",
                    d.encoding
                );
                assert_eq!(round_trip(&bytes, Some(1252))?, bytes);
            }
        }
        Ok(())
    }

    #[test]
    fn a_file_that_is_only_a_bom_round_trips_to_only_a_bom()
    -> Result<(), Box<dyn std::error::Error>> {
        for bom in [
            &[0xEF, 0xBB, 0xBF][..],
            &[0xFF, 0xFE][..],
            &[0xFE, 0xFF][..],
        ] {
            assert_eq!(round_trip(bom, None)?, bom, "BOM-only file must survive");
        }
        let d = detect(&[0xEF, 0xBB, 0xBF], None);
        assert!(d.bom_present);
        assert!(!d.trailing_newline);
        Ok(())
    }

    #[test]
    fn empty_file_round_trips_to_empty() -> Result<(), Box<dyn std::error::Error>> {
        // Writing a BOM or a newline into someone's empty file is a harm.
        let d = detect(&[], Some(1252));
        assert_eq!(d.encoding, TextEncoding::Utf8);
        assert_eq!(d.line_ending, LineEnding::Lf);
        assert!(!d.trailing_newline);
        assert!(!d.bom_present);
        assert_eq!(round_trip(&[], Some(1252))?, Vec::<u8>::new());
        Ok(())
    }

    #[test]
    fn cp1252_refuses_unencodable_chars_exactly() {
        let d = det(TextEncoding::Ansi(1252), false);
        for ch in ['\u{1F600}', '\u{0100}'] {
            let Err(e) = encode(&ch.to_string(), d) else {
                panic!("must refuse {ch:?} instead of writing '?'");
            };
            assert_eq!(
                e,
                EncodeError::Unencodable {
                    code_point: ch,
                    codepage: 1252
                }
            );
        }
    }

    #[test]
    fn cp1252_special_chars_round_trip_to_their_own_byte() -> Result<(), Box<dyn std::error::Error>>
    {
        // All 27 defined 0x80..=0x9F slots, each back to its own byte.
        for b in [
            0x80u8, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x8B, 0x8C, 0x8E, 0x91,
            0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0x9B, 0x9C, 0x9E, 0x9F,
        ] {
            let text = decode(&[b], det(TextEncoding::Ansi(1252), false))?;
            assert_eq!(text.chars().count(), 1, "byte {b:#04x} maps to one char");
            assert_eq!(
                encode(&text, det(TextEncoding::Ansi(1252), false))?,
                vec![b]
            );
            assert_eq!(round_trip(&[b], Some(1252))?, vec![b], "byte {b:#04x}");
        }
        // A Latin-1 identity byte, for completeness.
        assert_eq!(round_trip(&[0xFF], Some(1252))?, vec![0xFF]);
        Ok(())
    }

    #[test]
    fn cp1252_undefined_slots_are_refused() {
        let d = det(TextEncoding::Ansi(1252), false);
        for b in [0x81u8, 0x8D, 0x8F, 0x90, 0x9D] {
            let Err(e) = decode(&[b], d) else {
                panic!("undefined slot {b:#04x} must be refused, not replaced");
            };
            assert_eq!(e, DecodeError::UnsupportedCodepage(1252));
            let Err(e) = round_trip(&[b], Some(1252)) else {
                panic!("undefined slot {b:#04x} must fail the round trip");
            };
            assert_eq!(e, DecodeError::UnsupportedCodepage(1252));
        }
    }

    #[test]
    fn unsupported_codepage_is_refused_not_guessed() {
        // A non-UTF-8 file with a code page we do not implement: detect
        // reports it, decode refuses, the caller goes read-only.
        let bytes = [0x8A, 0x93]; // invalid utf-8, would be mojibake in 1252-terms
        let d = detect(&bytes, Some(1250));
        assert_eq!(d.encoding, TextEncoding::Ansi(1250));
        assert_eq!(
            decode(&bytes, d),
            Err(DecodeError::UnsupportedCodepage(1250))
        );
        // Encoding to an unsupported code page refuses the first character.
        assert_eq!(
            encode("x", det(TextEncoding::Ansi(1250), false)),
            Err(EncodeError::Unencodable {
                code_point: 'x',
                codepage: 1250
            })
        );
        // With NO code page supplied, an invalid-utf-8 file refuses as
        // malformed utf-8 — never a silent 1252 guess.
        assert_eq!(
            decode(&bytes, detect(&bytes, None)),
            Err(DecodeError::MalformedUtf8 { byte_offset: 0 })
        );
    }

    #[test]
    fn is_oversize_boundary_is_at_not_above() {
        assert!(!is_oversize(MAX_TEXT_BYTES - 1));
        assert!(is_oversize(MAX_TEXT_BYTES));
        assert!(is_oversize(MAX_TEXT_BYTES + 1));
    }
}
