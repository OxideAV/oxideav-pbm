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
    let w = h.width as usize;
    let hh = h.height as usize;
    let depth = h.depth as usize;
    let total_samples = w * hh * depth;
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

    Ok(DecodedSamples {
        width: h.width,
        height: h.height,
        depth: h.depth,
        maxval: h.maxval,
        samples: out,
    })
}

/// Encode a P1/P2/P3 ASCII body. Always emits one sample per line for
/// determinism (matches the canonical "plain" Netpbm output).
pub fn encode_ascii_body(samples: &[u16], width: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 4);
    let w = width as usize;
    for (i, &s) in samples.iter().enumerate() {
        // Group line breaks per pixel-column for readability — keeps
        // long lines from blowing past the 70-byte recommendation in
        // the man page (which itself only suggests, doesn't require).
        if i > 0 && i % w == 0 {
            out.push(b'\n');
        } else if i > 0 {
            out.push(b' ');
        }
        out.extend_from_slice(s.to_string().as_bytes());
    }
    out.push(b'\n');
    out
}

// ---------------------------------------------------------------------------
// Local copies of the header.rs whitespace+comment helpers so the body
// parser doesn't have to re-export them. Body and header use the same
// tokenization rules per the spec, including comment tolerance.
// ---------------------------------------------------------------------------

fn next_uint(input: &[u8], cursor: &mut usize) -> Result<u32> {
    skip_ws_and_comments(input, cursor);
    let start = *cursor;
    while *cursor < input.len() && input[*cursor].is_ascii_digit() {
        *cursor += 1;
    }
    if *cursor == start {
        return Err(Error::invalid(
            "Netpbm ASCII: expected decimal integer in body",
        ));
    }
    let s = std::str::from_utf8(&input[start..*cursor]).expect("ASCII digits");
    s.parse::<u32>()
        .map_err(|e| Error::invalid(format!("Netpbm ASCII: bad integer '{s}': {e}")))
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
    fn decode_p3_clamps_above_maxval() {
        let buf = b"P3\n1 1\n100\n200 50 75\n";
        let h = parse_header(buf).unwrap();
        let d = decode_ascii(&h, &buf[h.data_offset..]).unwrap();
        assert_eq!(d.samples, vec![100, 50, 75]);
    }
}
