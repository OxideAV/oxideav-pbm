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
    /// `Pf` — single-channel (grayscale) Portable FloatMap: raw
    /// IEEE-754 binary32 samples with a three-line header.
    PfPfmGrayFloat,
    /// `PF` — 3-channel (RGB) Portable FloatMap: raw IEEE-754 binary32
    /// samples with a three-line header.
    PFPfmRgbFloat,
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
            // Portable FloatMap: capital `F` = 3-channel RGB, lowercase
            // `f` = single-channel grayscale (case-sensitive).
            b'F' => Self::PFPfmRgbFloat,
            b'f' => Self::PfPfmGrayFloat,
            _ => return None,
        })
    }

    /// `true` for the two Portable FloatMap magics (`Pf` / `PF`), whose
    /// header shape and body encoding differ entirely from PNM/PAM (a
    /// fixed three-line comment-free header followed by raw IEEE-754
    /// binary32 samples in bottom-to-top order).
    pub fn is_pfm(self) -> bool {
        matches!(self, Self::PfPfmGrayFloat | Self::PFPfmRgbFloat)
    }

    /// `true` for P1/P2/P3 — sample data is whitespace-separated decimal
    /// integers and comments may appear anywhere up to EOF.
    pub fn is_ascii(self) -> bool {
        matches!(
            self,
            Self::P1AsciiBitmap | Self::P2AsciiGraymap | Self::P3AsciiPixmap
        )
    }

    /// `true` for the binary-body magics — `P4` / `P5` / `P6` / `P7` and
    /// the two Portable FloatMap magics (`Pf` / `PF`). Symmetric to
    /// [`Magic::is_ascii`]: every magic is either ASCII-bodied or
    /// binary-bodied, so `m.is_binary() == !m.is_ascii()` for every
    /// recognised value. Carved out as a typed predicate so call sites
    /// no longer need to enumerate the four (or six, including PFM)
    /// binary magic variants by hand to make routing decisions.
    pub fn is_binary(self) -> bool {
        !self.is_ascii()
    }

    /// `true` for the classic seven PNM/PAM magics (`P1` … `P7`) — the
    /// integer-sample family the Netpbm man pages cover. `false` for the
    /// two Portable FloatMap magics (`Pf` / `PF`), whose IEEE-754
    /// binary32 samples and three-line comment-free header live in a
    /// distinct spec (the Debevec PFM reference). Useful as a typed
    /// dispatch hinge: PNM magics share the integer `MAXVAL` /
    /// `bits_per_sample == {1, 8, 16}` shape; PFM magics don't.
    pub fn is_pnm(self) -> bool {
        !self.is_pfm()
    }

    /// Canonical on-disk magic bytes — the exact ASCII identifier the
    /// Netpbm spec puts at the start of every file (`b"P1"` … `b"P7"`,
    /// `b"Pf"`, `b"PF"`). Mirrors [`Magic::from_bytes`] in the opposite
    /// direction so an encoder can write the magic without re-deriving
    /// the digit / case from the variant by hand, and so a round-trip
    /// caller can assert `Magic::from_bytes(m.wire_bytes()) == Some(m)`.
    ///
    /// Returns a `&'static [u8]` because every variant maps to a fixed
    /// 2-byte literal; no allocation is required.
    pub fn wire_bytes(self) -> &'static [u8] {
        match self {
            Self::P1AsciiBitmap => b"P1",
            Self::P2AsciiGraymap => b"P2",
            Self::P3AsciiPixmap => b"P3",
            Self::P4BinaryBitmap => b"P4",
            Self::P5BinaryGraymap => b"P5",
            Self::P6BinaryPixmap => b"P6",
            Self::P7Pam => b"P7",
            Self::PfPfmGrayFloat => b"Pf",
            Self::PFPfmRgbFloat => b"PF",
        }
    }

    /// Channels-per-pixel implied by the magic. PAM is variable so this
    /// only covers P1-P6.
    pub fn channels(self) -> Option<usize> {
        Some(match self {
            Self::P1AsciiBitmap | Self::P4BinaryBitmap => 1,
            Self::P2AsciiGraymap | Self::P5BinaryGraymap => 1,
            Self::P3AsciiPixmap | Self::P6BinaryPixmap => 3,
            Self::PfPfmGrayFloat => 1,
            Self::PFPfmRgbFloat => 3,
            Self::P7Pam => return None,
        })
    }
}

/// PAM tuple type. The man page enumerates six standard names with fixed
/// channel counts; the PAM format spec also explicitly permits arbitrary
/// **user-defined** names (e.g. `DEPTH_MAP`, `RGBE`, `NORMAL_MAP`,
/// `OPACITY`, scientific multi-channel volumes), in which case the
/// caller is responsible for interpreting the channels — we round-trip
/// the name verbatim and route the pixels through the depth-based
/// fallback used when `TUPLTYPE` is omitted entirely.
///
/// `Custom(_)` always implies "no standard semantic" — the channel
/// count is whatever the header's `DEPTH` says (1..=4) and the decoder
/// falls back to opaque-gray / gray-alpha / RGB / RGBA based on that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tupltype {
    BlackAndWhite,
    Grayscale,
    Rgb,
    BlackAndWhiteAlpha,
    GrayscaleAlpha,
    RgbAlpha,
    /// A non-standard / user-defined tuple-type name. Preserved verbatim
    /// for round-trip; channel layout is determined by `DEPTH` alone.
    Custom(String),
}

impl Tupltype {
    /// Parse a TUPLTYPE name. Recognises the six standard names; any
    /// other non-empty ASCII token round-trips as [`Tupltype::Custom`].
    pub fn parse(name: &str) -> Result<Self> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(Error::invalid("PAM: TUPLTYPE value is empty"));
        }
        Ok(match trimmed {
            "BLACKANDWHITE" => Self::BlackAndWhite,
            "GRAYSCALE" => Self::Grayscale,
            "RGB" => Self::Rgb,
            "BLACKANDWHITE_ALPHA" => Self::BlackAndWhiteAlpha,
            "GRAYSCALE_ALPHA" => Self::GrayscaleAlpha,
            "RGB_ALPHA" => Self::RgbAlpha,
            other => Self::Custom(other.to_string()),
        })
    }

    /// Wire-name for the tuple type. Borrows the inner `String` for the
    /// custom case.
    pub fn name(&self) -> &str {
        match self {
            Self::BlackAndWhite => "BLACKANDWHITE",
            Self::Grayscale => "GRAYSCALE",
            Self::Rgb => "RGB",
            Self::BlackAndWhiteAlpha => "BLACKANDWHITE_ALPHA",
            Self::GrayscaleAlpha => "GRAYSCALE_ALPHA",
            Self::RgbAlpha => "RGB_ALPHA",
            Self::Custom(s) => s.as_str(),
        }
    }

    /// Channels-per-pixel implied by the **standard** tuple type. The
    /// custom case returns `None` — the caller falls back to the
    /// header's `DEPTH` field, which is the only authoritative source
    /// for arbitrary tuple-type files.
    pub fn channels(&self) -> Option<usize> {
        Some(match self {
            Self::BlackAndWhite | Self::Grayscale => 1,
            Self::BlackAndWhiteAlpha | Self::GrayscaleAlpha => 2,
            Self::Rgb => 3,
            Self::RgbAlpha => 4,
            Self::Custom(_) => return None,
        })
    }

    /// `true` for [`Tupltype::Custom`] — a non-standard tuple-type name
    /// that should round-trip verbatim but doesn't pin a channel layout.
    pub fn is_custom(&self) -> bool {
        matches!(self, Self::Custom(_))
    }
}

/// Portable FloatMap header metadata, populated only for the `Pf` / `PF`
/// magics. The third PFM header line carries both the byte order (via its
/// sign) and an application-defined scale factor (its absolute value).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PfmInfo {
    /// `true` when the on-disk float samples are little-endian (the
    /// header's scale line was negative); `false` for big-endian (a
    /// positive scale line).
    pub little_endian: bool,
    /// The absolute value of the scale line — an application-defined
    /// scale factor the producer associated with the samples. The
    /// decoder preserves the raw sample values unchanged and reports this
    /// magnitude as metadata; it does not apply it to the pixels.
    pub scale: f32,
}

/// Parsed header — common across all seven Netpbm magic numbers plus the
/// two Portable FloatMap magics.
#[derive(Debug, Clone)]
pub struct Header {
    pub magic: Magic,
    pub width: u32,
    pub height: u32,
    /// Maximum sample value. `1` for P1/P4 (implicit), 1..=65535 for the
    /// rest. Values > 255 force 16-bit big-endian binary samples on P5/P6/P7.
    /// Unused (`0`) for the Portable FloatMap magics, whose samples are
    /// IEEE-754 binary32 with no integer `MAXVAL`.
    pub maxval: u32,
    /// Channel count: derived from `magic` for P1-P6 (and `Pf` / `PF`)
    /// and read from `DEPTH` for P7.
    pub depth: u32,
    /// Only populated for P7. `None` for P1-P6 and the PFM magics.
    pub tupltype: Option<Tupltype>,
    /// Byte offset where the pixel data begins (0-based, into the input
    /// slice the header was parsed from).
    pub data_offset: usize,
    /// Byte order + scale metadata for the Portable FloatMap magics;
    /// `None` for every PNM/PAM magic.
    pub pfm: Option<PfmInfo>,
}

impl Header {
    /// Bits per sample on disk:
    /// * 1 for P1/P4 (the bit-packed/ASCII bitmap formats)
    /// * 8 for `maxval <= 255`
    /// * 16 for `maxval > 255`
    pub fn bits_per_sample(&self) -> u32 {
        match self.magic {
            Magic::P1AsciiBitmap | Magic::P4BinaryBitmap => 1,
            // IEEE-754 binary32 samples.
            Magic::PfPfmGrayFloat | Magic::PFPfmRgbFloat => 32,
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

/// Iterator over `# … LF` comment lines found in the **PNM/PAM** portion
/// of a Netpbm input (P1-P7 magics). Each item is the comment's *body*
/// — the bytes between the leading `#` and the terminating LF —
/// `ASCII`-trimmed at both ends.
///
/// Per `pbm(5)` / `pgm(5)` / `ppm(5)` / `pnm(5)` / `pam(5)`, a comment is
/// a line whose first non-blank byte is `#`, terminated by the next LF
/// (`0x0a`). The man pages permit comments anywhere up to the start of
/// the pixel data for P1-P6, and anywhere within the line-based PAM
/// header for P7 (where blank lines and comment lines may interleave
/// the `KEY VALUE` block before the terminating `ENDHDR`). The decoder
/// already tolerates them silently — this iterator surfaces them as a
/// typed accessor so a caller (e.g. an image-tool that needs to round
/// through producer metadata or a converter that wants to forward
/// "Created by …" provenance into a different container) can read them
/// without re-walking the header bytes.
///
/// The Portable FloatMap magics (`Pf` / `PF`) explicitly forbid comments
/// in their three-line header (per the Debevec reference); this iterator
/// yields **nothing** for a PFM input, matching the strict parser's
/// behaviour. An input whose magic is unrecognised likewise yields
/// nothing.
///
/// The iterator stops at the first byte of the pixel data — for P1-P6
/// that is the byte after the whitespace separator that follows the
/// `MAXVAL` (or `HEIGHT` for P1/P4) token, and for P7 that is the byte
/// after the LF following `ENDHDR`. Body comments inside a P1/P2/P3
/// ASCII pixel stream are *not* yielded — those are body-tokenizer
/// concern, distinct from the header-level comment surface this
/// accessor exposes.
///
/// Yielding `&[u8]` rather than `&str` matches the rest of the crate's
/// byte-oriented API and avoids forcing the caller to accept the
/// (rare-but-legal) case of a non-UTF-8 comment payload as an error;
/// callers that want a string can run [`std::str::from_utf8`] themselves.
#[derive(Debug)]
pub struct PnmHeaderComments<'a> {
    input: &'a [u8],
    cursor: usize,
    end: usize,
}

impl<'a> PnmHeaderComments<'a> {
    /// Return a fresh iterator that yields nothing.
    #[inline]
    fn empty() -> Self {
        Self {
            input: &[],
            cursor: 0,
            end: 0,
        }
    }
}

impl<'a> Iterator for PnmHeaderComments<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        while self.cursor < self.end {
            let c = self.input[self.cursor];
            if c == b'#' {
                // Found a `#` at the start of a token. Consume it, then
                // scan to the next LF (or `end` — a header missing its
                // final LF still gets walked to the boundary).
                let body_start = self.cursor + 1;
                let mut i = body_start;
                while i < self.end && self.input[i] != b'\n' {
                    i += 1;
                }
                let raw = &self.input[body_start..i];
                // Step past the LF terminator (if any).
                self.cursor = if i < self.end { i + 1 } else { i };
                return Some(trim_ascii(raw));
            }
            self.cursor += 1;
        }
        None
    }
}

/// Iterate the `# … LF` comment lines found in the header portion of
/// `input`, yielding each comment's text (trimmed of surrounding ASCII
/// whitespace) as `&[u8]`. See [`PnmHeaderComments`] for the precise
/// boundary and forbidden-magic rules.
///
/// This is a non-allocating accessor: the iterator borrows slices into
/// `input` directly. An input with no recognisable header (or a PFM
/// input) yields zero items.
pub fn iter_pnm_header_comments(input: &[u8]) -> PnmHeaderComments<'_> {
    let header = match parse_header(input) {
        Ok(h) => h,
        Err(_) => return PnmHeaderComments::empty(),
    };
    // PFM forbids comments by spec; never walk a PFM header looking for them.
    if header.magic.is_pfm() {
        return PnmHeaderComments::empty();
    }
    // The pixel data starts at `data_offset`; everything before that is
    // header bytes the comment scanner is allowed to touch.
    let end = header.data_offset.min(input.len());
    PnmHeaderComments {
        input,
        cursor: 0,
        end,
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
    if magic.is_pfm() {
        return parse_pfm_header(input, magic);
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
        pfm: None,
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
    if let Some(t) = &tupltype {
        // Only the six standard tuple-types pin a channel count. A
        // user-defined `Custom(_)` name is honoured at whatever `DEPTH`
        // the header advertises (1..=4, already range-checked above).
        if let Some(want) = t.channels() {
            if want as u32 != depth {
                return Err(Error::invalid(format!(
                    "PAM: TUPLTYPE {} expects depth {}, header says {depth}",
                    t.name(),
                    want
                )));
            }
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
        pfm: None,
    })
}

/// Parse a Portable FloatMap header: exactly three LF-terminated lines
/// (magic, "width height", scale) with **no comments** and **no CRLF**.
/// Per the Debevec PFM reference, each header line ends with a single
/// `0x0a` (LF), not the DOS `0x0d 0x0a` pair, and the format defines no
/// `#` comment syntax (unlike the PNM family).
fn parse_pfm_header(input: &[u8], magic: Magic) -> Result<Header> {
    let mut cursor = 0usize;

    // Line 1 — the magic / type token. Must be exactly `PF` (3-channel
    // RGB) or `Pf` (single-channel grayscale).
    let line1 = read_pfm_line(input, &mut cursor)?;
    let channels: u32 = match trim_ascii(line1) {
        b"PF" => 3,
        b"Pf" => 1,
        _ => return Err(Error::invalid("PFM: header line 1 is not 'PF' or 'Pf'")),
    };

    // Line 2 — `width height` as decimal ASCII integers.
    let line2 = read_pfm_line(input, &mut cursor)?;
    let (width, height) = parse_two_uints(line2)?;
    if width == 0 || height == 0 {
        return Err(Error::invalid("PFM: zero width or height"));
    }

    // Line 3 — the scale / endianness line: sign selects byte order
    // (negative = little-endian, positive = big-endian) and the absolute
    // value is the scale factor.
    let line3 = read_pfm_line(input, &mut cursor)?;
    let scale = parse_scale(line3)?;

    Ok(Header {
        magic,
        width,
        height,
        maxval: 0,
        depth: channels,
        tupltype: None,
        data_offset: cursor,
        pfm: Some(PfmInfo {
            little_endian: scale.is_sign_negative(),
            scale: scale.abs(),
        }),
    })
}

/// Read one Portable FloatMap header line: bytes up to (and consuming)
/// the next `0x0a`. Rejects an embedded `0x0d` (CRLF / stray CR), a `#`
/// (PFM defines no comments), and a missing LF terminator.
fn read_pfm_line<'a>(input: &'a [u8], cursor: &mut usize) -> Result<&'a [u8]> {
    if *cursor >= input.len() {
        return Err(Error::invalid("PFM: header truncated"));
    }
    let start = *cursor;
    while *cursor < input.len() {
        match input[*cursor] {
            b'\n' => {
                let line = &input[start..*cursor];
                *cursor += 1; // step past the LF
                return Ok(line);
            }
            b'\r' => {
                return Err(Error::invalid(
                    "PFM: carriage return in header (CRLF line endings are not allowed)",
                ))
            }
            b'#' => {
                return Err(Error::invalid(
                    "PFM: '#' in header (comments are not allowed)",
                ))
            }
            _ => *cursor += 1,
        }
    }
    Err(Error::invalid("PFM: header line missing LF terminator"))
}

/// Parse exactly two whitespace-separated decimal integers from a PFM
/// dimension line.
fn parse_two_uints(line: &[u8]) -> Result<(u32, u32)> {
    let s =
        std::str::from_utf8(line).map_err(|_| Error::invalid("PFM: non-UTF-8 dimension line"))?;
    let mut it = s.split_ascii_whitespace();
    let w = it
        .next()
        .ok_or_else(|| Error::invalid("PFM: missing width on dimension line"))?;
    let h = it
        .next()
        .ok_or_else(|| Error::invalid("PFM: missing height on dimension line"))?;
    if it.next().is_some() {
        return Err(Error::invalid("PFM: extra tokens on dimension line"));
    }
    Ok((parse_uint(w)?, parse_uint(h)?))
}

/// Parse the PFM scale / endianness line as an IEEE-754 binary32 value.
/// A `NaN` is rejected because its sign cannot disambiguate byte order.
fn parse_scale(line: &[u8]) -> Result<f32> {
    let s = std::str::from_utf8(line).map_err(|_| Error::invalid("PFM: non-UTF-8 scale line"))?;
    let v: f32 = s
        .trim()
        .parse()
        .map_err(|e| Error::invalid(format!("PFM: bad scale '{}': {e}", s.trim())))?;
    if v.is_nan() {
        return Err(Error::invalid("PFM: scale is NaN (ambiguous byte order)"));
    }
    Ok(v)
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

    #[test]
    fn accepts_user_defined_tupltype() {
        // PAM lets the producer name an arbitrary TUPLTYPE (depth maps,
        // RGBE light probes, scientific multi-channel volumes, ...).
        // We round-trip the name verbatim and let DEPTH drive the
        // channel count.
        let buf = b"P7\nWIDTH 2\nHEIGHT 1\nDEPTH 3\nMAXVAL 255\nTUPLTYPE DEPTH_MAP\nENDHDR\nABCDEF";
        let h = parse_header(buf).unwrap();
        assert_eq!(h.magic, Magic::P7Pam);
        assert_eq!(h.depth, 3);
        assert_eq!(h.maxval, 255);
        match &h.tupltype {
            Some(Tupltype::Custom(s)) => assert_eq!(s, "DEPTH_MAP"),
            other => panic!("expected Custom(DEPTH_MAP), got {other:?}"),
        }
        assert!(h.tupltype.as_ref().unwrap().is_custom());
        assert_eq!(h.tupltype.as_ref().unwrap().channels(), None);
        assert_eq!(h.tupltype.as_ref().unwrap().name(), "DEPTH_MAP");
        assert_eq!(&buf[h.data_offset..], b"ABCDEF");
    }

    #[test]
    fn custom_tupltype_with_any_depth_in_range() {
        // depth=4 → routes through depth-fallback (no Custom <-> depth check).
        let buf =
            b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGBE\nENDHDR\n\x01\x02\x03\x04";
        let h = parse_header(buf).unwrap();
        assert_eq!(h.tupltype.as_ref().map(|t| t.name()), Some("RGBE"));
        assert_eq!(h.depth, 4);
    }

    #[test]
    fn rejects_empty_tupltype_value() {
        // Empty TUPLTYPE is a malformed header — not a Custom("") variant.
        let buf = b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 1\nMAXVAL 255\nTUPLTYPE\nENDHDR\n\x00";
        let h = parse_header(buf);
        assert!(h.is_err(), "expected error, got {h:?}");
    }

    #[test]
    fn parses_pfm_rgb_little_endian_header() {
        let buf = b"PF\n4 3\n-1.0\n\x00\x00\x00\x00";
        let h = parse_header(buf).unwrap();
        assert_eq!(h.magic, Magic::PFPfmRgbFloat);
        assert_eq!(h.width, 4);
        assert_eq!(h.height, 3);
        assert_eq!(h.depth, 3);
        let info = h.pfm.expect("pfm metadata");
        assert!(info.little_endian);
        assert_eq!(info.scale, 1.0);
        // Header is "PF\n4 3\n-1.0\n" = 12 bytes; the lone payload byte
        // follows.
        assert_eq!(h.data_offset, 12);
    }

    #[test]
    fn parses_pfm_gray_big_endian_header_with_scale() {
        let buf = b"Pf\n2 2\n2.5\nbody";
        let h = parse_header(buf).unwrap();
        assert_eq!(h.magic, Magic::PfPfmGrayFloat);
        assert_eq!(h.depth, 1);
        let info = h.pfm.expect("pfm metadata");
        assert!(!info.little_endian);
        assert_eq!(info.scale, 2.5);
        assert_eq!(&buf[h.data_offset..], b"body");
    }

    #[test]
    fn pfm_rejects_crlf_in_header() {
        let buf = b"PF\r\n4 3\r\n-1.0\r\n";
        assert!(parse_header(buf).is_err());
    }

    #[test]
    fn pfm_rejects_comment_in_header() {
        let buf = b"PF\n# a comment\n4 3\n-1.0\n";
        assert!(parse_header(buf).is_err());
    }

    #[test]
    fn pfm_rejects_nan_scale() {
        let buf = b"Pf\n2 2\nNaN\n";
        assert!(parse_header(buf).is_err());
    }

    #[test]
    fn pfm_rejects_zero_dimension() {
        let buf = b"Pf\n0 2\n-1.0\n";
        assert!(parse_header(buf).is_err());
    }

    #[test]
    fn iter_comments_p4_yields_single_header_comment() {
        // The same fixture as `parses_p4_header_with_comments` — the
        // PNM header carries one `# created by GIMP` comment between
        // the magic and the dimensions line.
        let buf = b"P4\n# created by GIMP\n8 4\n\xFF\x00\xFF\x00";
        let comments: Vec<&[u8]> = iter_pnm_header_comments(buf).collect();
        assert_eq!(comments, vec![&b"created by GIMP"[..]]);
    }

    #[test]
    fn iter_comments_p3_walks_every_header_comment() {
        // Multiple comments interleaved with the magic / dimensions /
        // maxval tokens. The iterator must yield them in order, trim
        // surrounding whitespace, and stop at the start of the pixel
        // data (so the body comment between samples is *not* yielded —
        // it lives past `data_offset` for the ASCII case).
        let buf = b"P3\n# first\n#  second  \n2 1 # inline tail\n255\n0 0 0 1 1 1\n";
        let comments: Vec<&[u8]> = iter_pnm_header_comments(buf).collect();
        // The header-region comments end at the LF following the
        // maxval `255`; the `# inline tail` after `1` is also part of
        // the header (it precedes the maxval token), so it is yielded.
        assert_eq!(
            comments,
            vec![&b"first"[..], &b"second"[..], &b"inline tail"[..],]
        );
    }

    #[test]
    fn iter_comments_p7_walks_pam_block_comments() {
        // PAM is line-based and explicitly tolerates blank lines and
        // `# …` comment lines inside the KEY VALUE block before
        // ENDHDR. The iterator yields each comment text trimmed.
        let buf = b"P7\n# tool: oxideav-pbm\nWIDTH 2\n#       resolution note\nHEIGHT 1\nDEPTH 3\nMAXVAL 255\nTUPLTYPE RGB\nENDHDR\n\x00\x00\x00\x00\x00\x00";
        let comments: Vec<&[u8]> = iter_pnm_header_comments(buf).collect();
        assert_eq!(
            comments,
            vec![&b"tool: oxideav-pbm"[..], &b"resolution note"[..]]
        );
    }

    #[test]
    fn iter_comments_pfm_yields_nothing() {
        // Portable FloatMap explicitly forbids comments. A well-formed
        // PFM input has none, and (per the strict parser) one bearing
        // a `#` in the header is rejected; the iterator surfaces the
        // spec rule by yielding nothing for both cases.
        let buf = b"PF\n4 3\n-1.0\n\x00\x00\x00\x00";
        let comments: Vec<&[u8]> = iter_pnm_header_comments(buf).collect();
        assert_eq!(comments, Vec::<&[u8]>::new());
        // And the rejected (`#` in header) case yields nothing too,
        // because the header parse fails and the accessor falls back
        // to an empty iterator rather than walking a malformed input.
        let bad = b"PF\n# comment\n4 3\n-1.0\n";
        let comments: Vec<&[u8]> = iter_pnm_header_comments(bad).collect();
        assert_eq!(comments, Vec::<&[u8]>::new());
    }

    #[test]
    fn iter_comments_unrecognised_input_is_empty() {
        // Not a Netpbm magic at all → the iterator just yields nothing.
        let comments: Vec<&[u8]> = iter_pnm_header_comments(b"hello world").collect();
        assert_eq!(comments, Vec::<&[u8]>::new());
    }

    #[test]
    fn iter_comments_stops_at_pixel_data_for_binary_magic() {
        // A P5 header followed by raw 8-bit pixel data that happens to
        // contain a `#` byte (which is a perfectly valid sample
        // value, 0x23). The iterator must NOT misread that as a
        // comment — it stops at `data_offset`.
        let buf = b"P5\n# header\n2 1\n255\n\x23\x23";
        let comments: Vec<&[u8]> = iter_pnm_header_comments(buf).collect();
        assert_eq!(comments, vec![&b"header"[..]]);
    }

    #[test]
    fn magic_wire_bytes_round_trips_through_from_bytes() {
        // Every recognised variant must hand back its canonical on-disk
        // identifier, and `from_bytes` must accept it: the two halves of
        // the typed primitive form a closed loop. Catches accidental
        // mis-typed table entries (e.g. `b"P1"` vs `b"p1"`, swapped
        // `Pf` / `PF` case) at compile + test time so the encoder can
        // funnel every magic write through `wire_bytes()` without an
        // open-coded literal table.
        for m in [
            Magic::P1AsciiBitmap,
            Magic::P2AsciiGraymap,
            Magic::P3AsciiPixmap,
            Magic::P4BinaryBitmap,
            Magic::P5BinaryGraymap,
            Magic::P6BinaryPixmap,
            Magic::P7Pam,
            Magic::PfPfmGrayFloat,
            Magic::PFPfmRgbFloat,
        ] {
            let bytes = m.wire_bytes();
            assert_eq!(
                bytes.len(),
                2,
                "every Netpbm magic is exactly two bytes on disk"
            );
            assert_eq!(bytes[0], b'P', "Netpbm magic always starts with 'P'");
            assert_eq!(
                Magic::from_bytes(bytes),
                Some(m),
                "wire_bytes ↔ from_bytes must round-trip for {m:?}"
            );
        }
    }

    #[test]
    fn magic_wire_bytes_case_sensitivity_for_pfm() {
        // `Pf` and `PF` differ only in case; the PFM spec is
        // case-sensitive (lowercase `f` = single-channel grayscale,
        // uppercase `F` = 3-channel RGB) so the typed accessor must
        // preserve both halves rather than collapsing them.
        assert_eq!(Magic::PfPfmGrayFloat.wire_bytes(), b"Pf");
        assert_eq!(Magic::PFPfmRgbFloat.wire_bytes(), b"PF");
        assert_ne!(
            Magic::PfPfmGrayFloat.wire_bytes(),
            Magic::PFPfmRgbFloat.wire_bytes()
        );
    }

    #[test]
    fn magic_is_binary_is_exact_complement_of_is_ascii() {
        // The typed predicate is symmetric with `is_ascii` — every
        // recognised magic is exactly one of the two. Encoders that
        // need to branch on body-shape (e.g. ASCII vs binary writers)
        // can use either; the symmetry test pins the contract so a
        // future variant added without updating one of the predicates
        // is caught here rather than at the call site.
        for m in [
            Magic::P1AsciiBitmap,
            Magic::P2AsciiGraymap,
            Magic::P3AsciiPixmap,
            Magic::P4BinaryBitmap,
            Magic::P5BinaryGraymap,
            Magic::P6BinaryPixmap,
            Magic::P7Pam,
            Magic::PfPfmGrayFloat,
            Magic::PFPfmRgbFloat,
        ] {
            assert_ne!(
                m.is_ascii(),
                m.is_binary(),
                "is_ascii and is_binary must partition the magic set for {m:?}"
            );
        }
        // Spot-check both sides of the partition explicitly.
        assert!(Magic::P1AsciiBitmap.is_ascii() && !Magic::P1AsciiBitmap.is_binary());
        assert!(Magic::P4BinaryBitmap.is_binary() && !Magic::P4BinaryBitmap.is_ascii());
        assert!(Magic::PfPfmGrayFloat.is_binary());
        assert!(Magic::PFPfmRgbFloat.is_binary());
    }

    #[test]
    fn magic_is_pnm_is_exact_complement_of_is_pfm() {
        // `is_pnm` is the typed dispatch hinge for "this magic is in the
        // integer P1..=P7 family"; it must complement `is_pfm` exactly
        // so callers can pick either side of the partition without
        // tripping on a future variant. Mirrors the
        // is_ascii ↔ is_binary symmetry above.
        for m in [
            Magic::P1AsciiBitmap,
            Magic::P2AsciiGraymap,
            Magic::P3AsciiPixmap,
            Magic::P4BinaryBitmap,
            Magic::P5BinaryGraymap,
            Magic::P6BinaryPixmap,
            Magic::P7Pam,
            Magic::PfPfmGrayFloat,
            Magic::PFPfmRgbFloat,
        ] {
            assert_ne!(
                m.is_pnm(),
                m.is_pfm(),
                "is_pnm and is_pfm must partition the magic set for {m:?}"
            );
        }
        // Spot-check the two non-PNM members (the only ones in the PFM
        // family) — every other variant lives on the PNM side.
        assert!(Magic::PfPfmGrayFloat.is_pfm() && !Magic::PfPfmGrayFloat.is_pnm());
        assert!(Magic::PFPfmRgbFloat.is_pfm() && !Magic::PFPfmRgbFloat.is_pnm());
        assert!(Magic::P7Pam.is_pnm() && !Magic::P7Pam.is_pfm());
    }

    #[test]
    fn standard_tupltype_channel_check_still_applies() {
        // DEPTH 4 with RGB (which pins 3 channels) is still rejected —
        // the Custom escape hatch doesn't loosen the standard-name check.
        let buf =
            b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB\nENDHDR\n\x00\x00\x00\x00";
        let h = parse_header(buf);
        assert!(h.is_err(), "expected error, got {h:?}");
    }
}
