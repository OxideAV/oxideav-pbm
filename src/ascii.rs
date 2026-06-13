//! ASCII (P1 / P2 / P3) sample decoding + encoding.
//!
//! ASCII bodies are whitespace-separated decimal integers — the same
//! tokenizer used for the header (which tolerates `# … LF` comments)
//! is reused here so a comment in the middle of a P2 body is silently
//! skipped, matching every Netpbm tool in the wild.
//!
//! P1 has a quirk: the man page allows two forms — single-bit tokens
//! (`0` / `1` separated by whitespace) **or** whitespace-free runs of
//! `0`/`1` digits. We accept both: the tokenizer either reads a
//! whole-number token of any length and treats each digit as a sample,
//! or skips a whitespace byte and goes again.

use crate::error::{PbmError as Error, Result};

use crate::binary::DecodedSamples;
use crate::header::{Header, Magic};

/// Decode the ASCII body of a P1/P2/P3 image.
pub fn decode_ascii(h: &Header, body: &[u8]) -> Result<DecodedSamples> {
    decode_ascii_consumed(h, body).map(|(s, _)| s)
}

/// Decode the ASCII body of a P1/P2/P3 image, additionally reporting how
/// many bytes of `body` were consumed up to and including the final
/// sample token (but *not* any trailing whitespace after it). The
/// multi-image stream decoder uses the consumed length to locate the
/// next concatenated image's magic; single-image [`decode_ascii`]
/// discards it.
pub fn decode_ascii_consumed(h: &Header, body: &[u8]) -> Result<(DecodedSamples, usize)> {
    let w = h.width as usize;
    let hh = h.height as usize;
    let depth = h.depth as usize;
    let total_samples = w
        .checked_mul(hh)
        .and_then(|v| v.checked_mul(depth))
        .ok_or_else(|| Error::invalid("Netpbm: dimension overflow"))?;
    // Each ASCII sample is at minimum one byte on disk (a single digit,
    // optionally followed by whitespace). A header that claims more
    // samples than the body could possibly contain is malformed — fail
    // before allocating the output buffer, otherwise a multi-billion
    // dimension claim OOMs the process. The `+ 1` accounts for the
    // last-sample-needs-no-trailing-separator case.
    if total_samples > body.len().saturating_add(1) {
        return Err(Error::invalid(
            "Netpbm ASCII: declared dimensions exceed body length",
        ));
    }
    let mut out: Vec<u16> = Vec::with_capacity(total_samples);

    let mut cursor = 0usize;
    match h.magic {
        Magic::P1AsciiBitmap => {
            // The spec lets a P1 body either be one digit per token OR
            // whitespace-free digit runs. Read the whole body byte-by-
            // byte: skip whitespace + comments, take exactly one digit,
            // append, repeat. This handles both styles uniformly.
            while out.len() < total_samples {
                skip_ws_and_comments(body, &mut cursor);
                if cursor >= body.len() {
                    return Err(Error::invalid("PBM ASCII: ran out of bytes"));
                }
                let c = body[cursor];
                cursor += 1;
                let bit = match c {
                    b'0' => 0u16,
                    b'1' => 1u16,
                    _ => {
                        return Err(Error::invalid(format!(
                            "PBM ASCII: expected '0'/'1', got {c:#x}"
                        )))
                    }
                };
                out.push(bit);
            }
        }
        Magic::P2AsciiGraymap | Magic::P3AsciiPixmap => {
            let mv = h.maxval;
            for _ in 0..total_samples {
                let v = next_uint(body, &mut cursor)?;
                if v > mv {
                    // Spec leaves over-maxval values unspecified; clamp
                    // (matches every implementation we've seen).
                    out.push(mv as u16);
                } else {
                    out.push(v as u16);
                }
            }
        }
        _ => {
            return Err(Error::invalid("decode_ascii called with binary magic"));
        }
    }

    Ok((
        DecodedSamples {
            width: h.width,
            height: h.height,
            depth: h.depth,
            maxval: h.maxval,
            samples: out,
        },
        cursor,
    ))
}

/// Encode a P1/P2/P3 ASCII body. Always emits one sample per line for
/// determinism (matches the canonical "plain" Netpbm output).
pub fn encode_ascii_body(samples: &[u16], width: u32) -> Vec<u8> {
    // Reserve enough headroom for the worst case (5 digits + separator
    // per u16 sample) so the writer never has to grow mid-loop.
    let mut out = Vec::with_capacity(samples.len() * 6 + 1);
    let w = width as usize;
    let mut col = 0usize;
    for &s in samples.iter() {
        if col != 0 {
            // Group line breaks per pixel-column for readability — keeps
            // long lines from blowing past the 70-byte recommendation in
            // the man page (which itself only suggests, doesn't require).
            if col == w {
                out.push(b'\n');
                col = 0;
            } else {
                out.push(b' ');
            }
        }
        write_u16_dec(&mut out, s);
        col += 1;
    }
    out.push(b'\n');
    out
}

/// Encode an 8-bit-per-sample ASCII body. Specialised entry point for
/// the P2 / P3 paths whose `PbmPixelFormat` is `Gray8` / `Rgb24` — the
/// samples are already a `&[u8]` plane slice, so widening through a
/// `Vec<u16>` and re-narrowing to ASCII (the path the generic
/// [`encode_ascii_body`] takes) is pure overhead. Writes digits straight
/// from the source bytes via a 256-entry lookup table.
pub(crate) fn encode_ascii_body_u8(samples: &[u8], stride_samples: usize) -> Vec<u8> {
    // Max ASCII width per sample is 3 digits + 1 separator. Add 1 for
    // the trailing LF.
    let mut out = Vec::with_capacity(samples.len() * 4 + 1);
    let mut col = 0usize;
    for &s in samples.iter() {
        if col != 0 {
            if col == stride_samples {
                out.push(b'\n');
                col = 0;
            } else {
                out.push(b' ');
            }
        }
        write_u8_dec(&mut out, s);
        col += 1;
    }
    out.push(b'\n');
    out
}

/// Encode a P1 ASCII bit body straight from `MonoBlack` row bytes.
/// Each output byte is `b'0'` or `b'1'` separated by a space (line
/// break at the row boundary). Skips the `Vec<u16>` widen step the
/// generic [`encode_ascii_body`] takes.
pub(crate) fn encode_ascii_body_bits(
    rows: &[u8],
    row_stride: usize,
    width: usize,
    height: usize,
) -> Vec<u8> {
    // Two ASCII bytes per pixel (digit + separator) plus a trailing LF.
    let mut out = Vec::with_capacity(width * height * 2 + 1);
    for y in 0..height {
        let row = &rows[y * row_stride..y * row_stride + row_stride];
        for x in 0..width {
            if x != 0 {
                out.push(b' ');
            }
            let bit = (row[x / 8] >> (7 - (x % 8))) & 1;
            out.push(b'0' + bit);
        }
        out.push(b'\n');
    }
    out
}

/// Append the decimal representation of a `u16` to `out` without a
/// heap allocation. The implementation writes through a 5-byte stack
/// scratch buffer (max width of `u16::MAX = 65535`).
#[inline]
fn write_u16_dec(out: &mut Vec<u8>, mut v: u16) {
    // Fast path: single digit (very common for clamped low-bit samples).
    if v < 10 {
        out.push(b'0' + v as u8);
        return;
    }
    // Fast path: most P2/P3 samples are in 0..=255 — use the u8 writer
    // which avoids the wide-u16 division entirely.
    if v < 256 {
        write_u8_dec(out, v as u8);
        return;
    }
    let mut buf = [0u8; 5];
    let mut i = buf.len();
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    out.extend_from_slice(&buf[i..]);
}

/// Append the decimal representation of a `u8` to `out`. Three branches
/// cover the 1 / 2 / 3 digit cases without a loop or division.
#[inline]
fn write_u8_dec(out: &mut Vec<u8>, v: u8) {
    if v < 10 {
        out.push(b'0' + v);
    } else if v < 100 {
        out.push(b'0' + v / 10);
        out.push(b'0' + v % 10);
    } else {
        out.push(b'0' + v / 100);
        out.push(b'0' + (v / 10) % 10);
        out.push(b'0' + v % 10);
    }
}

// ---------------------------------------------------------------------------
// Local copies of the header.rs whitespace+comment helpers so the body
// parser doesn't have to re-export them. Body and header use the same
// tokenization rules per the spec, including comment tolerance.
// ---------------------------------------------------------------------------

fn next_uint(input: &[u8], cursor: &mut usize) -> Result<u32> {
    skip_ws_and_comments(input, cursor);
    let start = *cursor;
    // Accumulate digits directly into a u32 — the UTF-8 + parse round
    // trip the previous implementation took was a measurable hot-path
    // tax for ASCII bodies (a 320x240 P3 spends most of its decode time
    // in this loop). Overflow guard mirrors `str::parse::<u32>`'s
    // behaviour: any value past `u32::MAX` is rejected.
    let bytes = input;
    let mut i = *cursor;
    let mut v: u32 = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if !c.is_ascii_digit() {
            break;
        }
        let d = (c - b'0') as u32;
        v = v
            .checked_mul(10)
            .and_then(|t| t.checked_add(d))
            .ok_or_else(|| Error::invalid("Netpbm ASCII: integer overflows u32"))?;
        i += 1;
    }
    if i == start {
        return Err(Error::invalid(
            "Netpbm ASCII: expected decimal integer in body",
        ));
    }
    *cursor = i;
    Ok(v)
}

fn skip_ws_and_comments(input: &[u8], cursor: &mut usize) {
    loop {
        while *cursor < input.len() && is_ws(input[*cursor]) {
            *cursor += 1;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::parse_header;

    #[test]
    fn decode_p1_packed_digits_no_whitespace() {
        let buf = b"P1\n4 2\n10100110\n";
        let h = parse_header(buf).unwrap();
        let d = decode_ascii(&h, &buf[h.data_offset..]).unwrap();
        assert_eq!(d.samples, vec![1, 0, 1, 0, 0, 1, 1, 0]);
    }

    #[test]
    fn decode_p2_with_inline_comment() {
        let buf = b"P2\n2 2\n255\n0 # half row comment\n128\n# midline\n200 50\n";
        let h = parse_header(buf).unwrap();
        let d = decode_ascii(&h, &buf[h.data_offset..]).unwrap();
        assert_eq!(d.samples, vec![0, 128, 200, 50]);
    }

    #[test]
    fn ascii_huge_dimension_does_not_oom() {
        // Regression: r171 fuzz target found a P2 input that claimed
        // width=2, height=200_000_000 with maxval=50 and only a few
        // bytes of body. The pre-fix decoder allocated
        // `Vec<u16>::with_capacity(400_000_000)` and the process
        // OOMed. The fix rejects the input upfront with InvalidData.
        let buf = b"P2\n2 200888808\n50\n0 0 0 0";
        let h = parse_header(buf).unwrap();
        let err = decode_ascii(&h, &buf[h.data_offset..]).unwrap_err();
        match err {
            crate::error::PbmError::InvalidData(s) => {
                assert!(
                    s.contains("declared dimensions exceed body length"),
                    "unexpected message: {s}"
                );
            }
            other => panic!("expected InvalidData, got {other:?}"),
        }
    }

    #[test]
    fn decode_p3_clamps_above_maxval() {
        let buf = b"P3\n1 1\n100\n200 50 75\n";
        let h = parse_header(buf).unwrap();
        let d = decode_ascii(&h, &buf[h.data_offset..]).unwrap();
        assert_eq!(d.samples, vec![100, 50, 75]);
    }

    #[test]
    fn ascii_body_round_trips_through_decoder() {
        // The optimised writer must produce a byte sequence that the
        // optimised reader can round-trip exactly. Use values that
        // exercise the 1 / 2 / 3 / 4-digit branches in `write_u16_dec`.
        let samples: Vec<u16> = vec![0, 9, 10, 99, 100, 999, 1000, 9999, 65535, 1];
        let body = encode_ascii_body(&samples, samples.len() as u32);
        // Manually feed it back through `next_uint` (the body has a
        // trailing LF the parser tolerates as whitespace).
        let mut cursor = 0usize;
        let mut got: Vec<u32> = Vec::new();
        while cursor < body.len() {
            skip_ws_and_comments(&body, &mut cursor);
            if cursor >= body.len() {
                break;
            }
            got.push(next_uint(&body, &mut cursor).unwrap());
        }
        let want: Vec<u32> = samples.iter().map(|&v| v as u32).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn ascii_decoder_rejects_overflow_integer() {
        // `next_uint` accumulates into a `u32`; a 13-digit run should
        // hit the `checked_mul`/`checked_add` overflow guard rather
        // than truncating silently.
        let buf = b"P2\n10 1\n65535\n12345678901234 0 0 0 0 0 0 0 0 0\n";
        let h = parse_header(buf).unwrap();
        let err = decode_ascii(&h, &buf[h.data_offset..]).unwrap_err();
        match err {
            crate::error::PbmError::InvalidData(s) => {
                assert!(s.contains("overflow") || s.contains("u32"), "got: {s}");
            }
            other => panic!("expected InvalidData, got {other:?}"),
        }
    }

    #[test]
    fn write_u8_dec_covers_all_digit_widths() {
        let cases: &[(u8, &[u8])] = &[
            (0, b"0"),
            (5, b"5"),
            (9, b"9"),
            (10, b"10"),
            (42, b"42"),
            (99, b"99"),
            (100, b"100"),
            (250, b"250"),
            (255, b"255"),
        ];
        for (v, want) in cases {
            let mut out = Vec::new();
            write_u8_dec(&mut out, *v);
            assert_eq!(out.as_slice(), *want, "u8 {v}");
        }
    }

    #[test]
    fn write_u16_dec_covers_all_digit_widths() {
        let cases: &[(u16, &[u8])] = &[
            (0, b"0"),
            (10, b"10"),
            (255, b"255"),
            (256, b"256"),
            (999, b"999"),
            (1000, b"1000"),
            (65535, b"65535"),
        ];
        for (v, want) in cases {
            let mut out = Vec::new();
            write_u16_dec(&mut out, *v);
            assert_eq!(out.as_slice(), *want, "u16 {v}");
        }
    }
}
