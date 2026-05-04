//! Netpbm header parsing — shared across PNM (P1-P6) and PAM (P7).
//!
//! The Netpbm family puts a tiny ASCII header in front of every image:
//!
//! * `P<N>\n<width> <height>\n[<maxval>\n]<pixel data>` for P1-P6
//! * a multi-line key/value block for P7 (PAM):
//!   ```text
//!   P7
//!   WIDTH 16
//!   HEIGHT 16
//!   DEPTH 4
//!   MAXVAL 255
//!   TUPLTYPE RGB_ALPHA
//!   ENDHDR
//!   <pixel data>
//!   ```
//!
//! Comments — lines starting with `#` after any preceding whitespace —
//! are tolerated anywhere in headers (and, for P1-P3 ASCII bodies, also
//! between samples). The man pages define no maximum line length; we
//! simply scan token-by-token until each numeric field is filled.

use crate::error::{PbmError as Error, Result};

/// One of the seven Netpbm magic numbers, plus PAM (P7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Magic {
    /// `P1` — ASCII PBM (1 bit, 1 = black).
    P1AsciiBitmap,
    /// `P2` — ASCII PGM (gray).
    P2AsciiGraymap,
    /// `P3` — ASCII PPM (RGB).
    P3AsciiPixmap,
    /// `P4` — binary PBM.
    P4BinaryBitmap,
    /// `P5` — binary PGM.
    P5BinaryGraymap,
    /// `P6` — binary PPM.
    P6BinaryPixmap,
    /// `P7` — binary PAM with multi-line header.
    P7Pam,
}

impl Magic {
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 2 || b[0] != b'P' {
            return None;
        }
        Some(match b[1] {
            b'1' => Self::P1AsciiBitmap,
            b'2' => Self::P2AsciiGraymap,
            b'3' => Self::P3AsciiPixmap,
            b'4' => Self::P4BinaryBitmap,
            b'5' => Self::P5BinaryGraymap,
            b'6' => Self::P6BinaryPixmap,
            b'7' => Self::P7Pam,
            _ => return None,
        })
    }

    /// `true` for P1/P2/P3 — sample data is whitespace-separated decimal
    /// integers and comments may appear anywhere up to EOF.
    pub fn is_ascii(self) -> bool {
        matches!(
            self,
            Self::P1AsciiBitmap | Self::P2AsciiGraymap | Self::P3AsciiPixmap
        )
    }

    /// Channels-per-pixel implied by the magic. PAM is variable so this
    /// only covers P1-P6.
    pub fn channels(self) -> Option<usize> {
        Some(match self {
            Self::P1AsciiBitmap | Self::P4BinaryBitmap => 1,
            Self::P2AsciiGraymap | Self::P5BinaryGraymap => 1,
            Self::P3AsciiPixmap | Self::P6BinaryPixmap => 3,
            Self::P7Pam => return None,
        })
    }
}

/// PAM tuple type. The man page enumerates six standard names; arbitrary
/// user-defined types are deferred to round 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tupltype {
    BlackAndWhite,
    Grayscale,
    Rgb,
    BlackAndWhiteAlpha,
    GrayscaleAlpha,
    RgbAlpha,
}

impl Tupltype {
    pub fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "BLACKANDWHITE" => Self::BlackAndWhite,
            "GRAYSCALE" => Self::Grayscale,
            "RGB" => Self::Rgb,
            "BLACKANDWHITE_ALPHA" => Self::BlackAndWhiteAlpha,
            "GRAYSCALE_ALPHA" => Self::GrayscaleAlpha,
            "RGB_ALPHA" => Self::RgbAlpha,
            other => {
                return Err(Error::unsupported(format!(
                    "PAM: tuple type '{other}' not supported in round 1"
                )))
            }
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::BlackAndWhite => "BLACKANDWHITE",
            Self::Grayscale => "GRAYSCALE",
            Self::Rgb => "RGB",
            Self::BlackAndWhiteAlpha => "BLACKANDWHITE_ALPHA",
            Self::GrayscaleAlpha => "GRAYSCALE_ALPHA",
            Self::RgbAlpha => "RGB_ALPHA",
        }
    }

    pub fn channels(self) -> usize {
        match self {
            Self::BlackAndWhite | Self::Grayscale => 1,
            Self::BlackAndWhiteAlpha | Self::GrayscaleAlpha | Self::Rgb => match self {
                Self::Rgb => 3,
                _ => 2,
            },
            Self::RgbAlpha => 4,
        }
    }
}

/// Parsed header — common across all seven magic numbers.
#[derive(Debug, Clone)]
pub struct Header {
    pub magic: Magic,
    pub width: u32,
    pub height: u32,
    /// Maximum sample value. `1` for P1/P4 (implicit), 1..=65535 for the
    /// rest. Values > 255 force 16-bit big-endian binary samples on P5/P6/P7.
    pub maxval: u32,
    /// Channel count: derived from `magic` for P1-P6 and read from
    /// `DEPTH` for P7.
    pub depth: u32,
    /// Only populated for P7. `None` for P1-P6.
    pub tupltype: Option<Tupltype>,
    /// Byte offset where the pixel data begins (0-based, into the input
    /// slice the header was parsed from).
    pub data_offset: usize,
}

impl Header {
    /// Bits per sample on disk:
    /// * 1 for P1/P4 (the bit-packed/ASCII bitmap formats)
    /// * 8 for `maxval <= 255`
    /// * 16 for `maxval > 255`
    pub fn bits_per_sample(&self) -> u32 {
        match self.magic {
            Magic::P1AsciiBitmap | Magic::P4BinaryBitmap => 1,
            _ => {
                if self.maxval > 255 {
                    16
                } else {
                    8
                }
            }
        }
    }
}

/// Parse a Netpbm header from the beginning of `input`. Returns the
/// fully-populated [`Header`] including `data_offset`, the byte index of
/// the first sample byte.
pub fn parse_header(input: &[u8]) -> Result<Header> {
    let magic = Magic::from_bytes(input)
        .ok_or_else(|| Error::invalid("Netpbm: missing or unrecognised P<N> magic"))?;
    if magic == Magic::P7Pam {
        return parse_pam_header(input);
    }
    parse_pnm_header(input, magic)
}

fn parse_pnm_header(input: &[u8], magic: Magic) -> Result<Header> {
    // After the 2-byte magic the spec mandates "whitespace (blanks, TABs,
    // CRs, LFs)" as the next byte — we just skip the magic and start the
    // tokenizer. Comments are handled by the tokenizer itself.
    let mut cursor = 2usize;
    let width = next_uint(input, &mut cursor)?;
    let height = next_uint(input, &mut cursor)?;
    let maxval = match magic {
        Magic::P1AsciiBitmap | Magic::P4BinaryBitmap => 1,
        _ => {
            let v = next_uint(input, &mut cursor)?;
            if !(1..=65535).contains(&v) {
                return Err(Error::invalid(format!(
                    "Netpbm: maxval {v} out of range 1..=65535"
                )));
            }
            v
        }
    };
    // Per the spec the byte immediately after `maxval` (or after the
    // height token for P1/P4) is exactly one whitespace byte; the next
    // byte is the first pixel byte. The tokenizer leaves `cursor` on the
    // whitespace byte itself for binary formats and we step past it.
    let data_offset = if magic.is_ascii() {
        // For ASCII formats the body is also whitespace-separated tokens
        // — leave the cursor where it is so the body parser can keep
        // tokenising from the same position. Comments inside the body
        // are tolerated by the same `next_uint` helper.
        cursor
    } else {
        // Binary: the spec requires exactly one whitespace byte
        // separator between the last header token and the pixel data,
        // and the tokenizer's `cursor` is already pointing one past
        // that separator after consuming the maxval token.
        cursor
    };
    Ok(Header {
        magic,
        width,
        height,
        maxval,
        depth: magic.channels().expect("P1-P6 have implicit channels") as u32,
        tupltype: None,
        data_offset,
    })
}

fn parse_pam_header(input: &[u8]) -> Result<Header> {
    // PAM is line-based. We accept LF or CRLF. Comments (lines whose first
    // non-blank byte is `#`) are skipped. The header ends at a line
    // containing exactly `ENDHDR`.
    let mut cursor = 2usize;
    // The 2-byte magic is followed by an optional CR and a mandatory LF.
    skip_eol(input, &mut cursor);

    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut depth: Option<u32> = None;
    let mut maxval: Option<u32> = None;
    let mut tupltype: Option<Tupltype> = None;

    loop {
        let line = read_line(input, &mut cursor)?;
        let trimmed = trim_ascii(line);
        if trimmed.is_empty() || trimmed.starts_with(b"#") {
            continue;
        }
        if trimmed == b"ENDHDR" {
            break;
        }
        // Each header line is `KEY <space> VALUE` — split on the first
        // run of ASCII whitespace.
        let (key, rest) = split_first_token(trimmed);
        let value = trim_ascii(rest);
        let value_str = std::str::from_utf8(value)
            .map_err(|_| Error::invalid("PAM: non-UTF-8 header value"))?;
        match key {
            b"WIDTH" => width = Some(parse_uint(value_str)?),
            b"HEIGHT" => height = Some(parse_uint(value_str)?),
            b"DEPTH" => depth = Some(parse_uint(value_str)?),
            b"MAXVAL" => {
                let v = parse_uint(value_str)?;
                if !(1..=65535).contains(&v) {
                    return Err(Error::invalid(format!(
                        "PAM: maxval {v} out of range 1..=65535"
                    )));
                }
                maxval = Some(v);
            }
            b"TUPLTYPE" => tupltype = Some(Tupltype::parse(value_str.trim())?),
            other => {
                // The man page says implementations should ignore
                // unknown keys for forward compatibility — be lenient.
                let _ = other;
            }
        }
    }

    let width = width.ok_or_else(|| Error::invalid("PAM: missing WIDTH"))?;
    let height = height.ok_or_else(|| Error::invalid("PAM: missing HEIGHT"))?;
    let depth = depth.ok_or_else(|| Error::invalid("PAM: missing DEPTH"))?;
    let maxval = maxval.ok_or_else(|| Error::invalid("PAM: missing MAXVAL"))?;
    if depth == 0 || depth > 4 {
        return Err(Error::unsupported(format!(
            "PAM: DEPTH {depth} out of round-1 range 1..=4"
        )));
    }
    if let Some(t) = tupltype {
        if t.channels() as u32 != depth {
            return Err(Error::invalid(format!(
                "PAM: TUPLTYPE {} expects depth {}, header says {depth}",
                t.name(),
                t.channels()
            )));
        }
    }
    Ok(Header {
        magic: Magic::P7Pam,
        width,
        height,
        maxval,
        depth,
        tupltype,
        data_offset: cursor,
    })
}

// ---------------------------------------------------------------------------
// Token / line / number helpers
// ---------------------------------------------------------------------------

/// Skip whitespace and `# … LF` comments, then read one decimal integer.
/// Leaves `cursor` pointing one byte past the last digit (which is the
/// terminating whitespace for binary formats and the start of the next
/// token / pixel for ASCII formats).
pub fn next_uint(input: &[u8], cursor: &mut usize) -> Result<u32> {
    skip_ws_and_comments(input, cursor);
    let start = *cursor;
    while *cursor < input.len() {
        let c = input[*cursor];
        if c.is_ascii_digit() {
            *cursor += 1;
        } else {
            break;
        }
    }
    if *cursor == start {
        return Err(Error::invalid(
            "Netpbm: expected decimal integer in header/body",
        ));
    }
    let s = std::str::from_utf8(&input[start..*cursor]).expect("ASCII digits");
    let v = s
        .parse::<u32>()
        .map_err(|e| Error::invalid(format!("Netpbm: bad integer '{s}': {e}")))?;
    // Skip the single mandatory whitespace byte that terminates the
    // token — for binary formats this is the separator between header
    // and pixel data, and for ASCII formats it just moves us onto the
    // next token (the loop in `next_uint` will skip any further
    // whitespace on the next call).
    if *cursor < input.len() && is_ws(input[*cursor]) {
        *cursor += 1;
    }
    Ok(v)
}

/// Like [`next_uint`] but returns `None` instead of an error when the
/// stream is exhausted (whitespace-only tail after the last token).
pub fn try_next_uint(input: &[u8], cursor: &mut usize) -> Result<Option<u32>> {
    skip_ws_and_comments(input, cursor);
    if *cursor >= input.len() {
        return Ok(None);
    }
    Ok(Some(next_uint(input, cursor)?))
}

fn skip_ws_and_comments(input: &[u8], cursor: &mut usize) {
    loop {
        // Whitespace.
        while *cursor < input.len() && is_ws(input[*cursor]) {
            *cursor += 1;
        }
        // Comment to end-of-line.
        if *cursor < input.len() && input[*cursor] == b'#' {
            while *cursor < input.len() && input[*cursor] != b'\n' {
                *cursor += 1;
            }
            continue;
        }
        break;
    }
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0B | 0x0C)
}

fn skip_eol(input: &[u8], cursor: &mut usize) {
    while *cursor < input.len() {
        let c = input[*cursor];
        *cursor += 1;
        if c == b'\n' {
            return;
        }
    }
}

fn read_line<'a>(input: &'a [u8], cursor: &mut usize) -> Result<&'a [u8]> {
    if *cursor >= input.len() {
        return Err(Error::invalid("PAM: header truncated (expected ENDHDR)"));
    }
    let start = *cursor;
    while *cursor < input.len() && input[*cursor] != b'\n' {
        *cursor += 1;
    }
    let end = *cursor;
    if *cursor < input.len() {
        // step past the LF
        *cursor += 1;
    }
    // Drop a trailing CR if present (CRLF case).
    let slice = &input[start..end];
    Ok(if slice.last() == Some(&b'\r') {
        &slice[..slice.len() - 1]
    } else {
        slice
    })
}

fn trim_ascii(b: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < b.len() && is_ws(b[i]) {
        i += 1;
    }
    let mut j = b.len();
    while j > i && is_ws(b[j - 1]) {
        j -= 1;
    }
    &b[i..j]
}

fn split_first_token(b: &[u8]) -> (&[u8], &[u8]) {
    let mut i = 0;
    while i < b.len() && !is_ws(b[i]) {
        i += 1;
    }
    let key = &b[..i];
    while i < b.len() && is_ws(b[i]) {
        i += 1;
    }
    (key, &b[i..])
}

fn parse_uint(s: &str) -> Result<u32> {
    s.trim()
        .parse::<u32>()
        .map_err(|e| Error::invalid(format!("PAM: bad integer '{s}': {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_p4_header_with_comments() {
        let buf = b"P4\n# created by GIMP\n8 4\n\xFF\x00\xFF\x00";
        let h = parse_header(buf).unwrap();
        assert_eq!(h.magic, Magic::P4BinaryBitmap);
        assert_eq!(h.width, 8);
        assert_eq!(h.height, 4);
        assert_eq!(h.maxval, 1);
        assert_eq!(h.depth, 1);
        assert_eq!(h.bits_per_sample(), 1);
        // First pixel byte should be 0xFF.
        assert_eq!(buf[h.data_offset], 0xFF);
    }

    #[test]
    fn parses_p6_header_16bit() {
        let buf = b"P6\n2 1\n65535\n\x00\x00\x00\x00\x00\x00";
        let h = parse_header(buf).unwrap();
        assert_eq!(h.magic, Magic::P6BinaryPixmap);
        assert_eq!(h.width, 2);
        assert_eq!(h.height, 1);
        assert_eq!(h.maxval, 65535);
        assert_eq!(h.depth, 3);
        assert_eq!(h.bits_per_sample(), 16);
    }

    #[test]
    fn parses_p7_pam_header() {
        let buf = b"P7\nWIDTH 4\nHEIGHT 2\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\nDATA";
        let h = parse_header(buf).unwrap();
        assert_eq!(h.magic, Magic::P7Pam);
        assert_eq!(h.width, 4);
        assert_eq!(h.height, 2);
        assert_eq!(h.depth, 4);
        assert_eq!(h.maxval, 255);
        assert_eq!(h.tupltype, Some(Tupltype::RgbAlpha));
        assert_eq!(&buf[h.data_offset..], b"DATA");
    }

    #[test]
    fn rejects_unknown_magic() {
        assert!(parse_header(b"P9\n2 2\n\x00\x00\x00\x00").is_err());
    }
}
