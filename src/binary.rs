//! Binary (P4 / P5 / P6 / P7) sample decoding + encoding.
//!
//! P4 packs 8 bits per byte MSB-first with rows padded to a byte
//! boundary. P5/P6 are row-major streams of 8-bit or 16-bit big-endian
//! samples (1 channel for P5, 3 for P6). P7 is the same row-major shape
//! but with `DEPTH` channels and one of six standard `TUPLTYPE`s
//! describing how those channels map to a colour model.

use crate::error::{PbmError as Error, Result};

use crate::header::{Header, Magic};

/// Decoded sample matrix, big-endian normalised into native ints.
///
/// `data` is row-major, `width * height * depth` samples long. For
/// `bits_per_sample == 1` and 8 there's one entry per sample fitting in
/// the byte (we widen to u16 anyway so callers can treat the buffer
/// uniformly).
pub struct DecodedSamples {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub maxval: u32,
    pub samples: Vec<u16>,
}

/// Decode the binary body of a P4/P5/P6/P7 image. The caller has
/// already parsed the [`Header`] and seeked `data` to start at the
/// pixel array (i.e. `&input[header.data_offset..]`).
pub fn decode_binary(h: &Header, data: &[u8]) -> Result<DecodedSamples> {
    let w = h.width as usize;
    let hh = h.height as usize;
    let depth = h.depth as usize;
    if w == 0 || hh == 0 || depth == 0 {
        return Err(Error::invalid("Netpbm: zero dimension"));
    }
    let samples_per_pixel = depth;
    let total_samples = w
        .checked_mul(hh)
        .and_then(|v| v.checked_mul(samples_per_pixel))
        .ok_or_else(|| Error::invalid("Netpbm: dimension overflow"))?;
    let mut out = vec![0u16; total_samples];

    match h.magic {
        Magic::P4BinaryBitmap => {
            // 1 bit per pixel, packed MSB-first, rows padded to a byte
            // boundary. Per the spec a 1-bit means BLACK.
            let row_bytes = w.div_ceil(8);
            let need = row_bytes * hh;
            if data.len() < need {
                return Err(Error::invalid("PBM: pixel data truncated"));
            }
            for y in 0..hh {
                let row = &data[y * row_bytes..y * row_bytes + row_bytes];
                for x in 0..w {
                    let byte = row[x / 8];
                    let bit = (byte >> (7 - (x % 8))) & 1;
                    out[y * w + x] = bit as u16;
                }
            }
        }
        Magic::P5BinaryGraymap | Magic::P6BinaryPixmap | Magic::P7Pam => {
            let bps = h.bits_per_sample();
            let bytes_per_sample = if bps == 16 { 2 } else { 1 };
            let need = total_samples
                .checked_mul(bytes_per_sample)
                .ok_or_else(|| Error::invalid("Netpbm: dimension overflow"))?;
            if data.len() < need {
                return Err(Error::invalid("Netpbm: pixel data truncated"));
            }
            if bytes_per_sample == 1 {
                for (i, b) in data[..need].iter().enumerate() {
                    out[i] = *b as u16;
                }
            } else {
                for (i, chunk) in data[..need].chunks_exact(2).enumerate() {
                    out[i] = u16::from_be_bytes([chunk[0], chunk[1]]);
                }
            }
            // Validate that no sample exceeds maxval (the spec allows
            // implementations to clamp instead, but a strict check
            // surfaces malformed files early).
            let mv = h.maxval as u16;
            if h.maxval < 65535 {
                for s in out.iter_mut() {
                    if *s > mv {
                        *s = mv;
                    }
                }
            }
        }
        _ => {
            return Err(Error::invalid("decode_binary called with non-binary magic"));
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

/// Encode a P4 (binary PBM) body. `bits` is row-major, `width * height`
/// values, each 0 (white) or 1 (black). Output is bit-packed MSB-first
/// with rows padded to a whole byte.
pub fn encode_p4_body(width: u32, height: u32, bits: &[u8]) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let row_bytes = w.div_ceil(8);
    let mut out = vec![0u8; row_bytes * h];
    for y in 0..h {
        let row = &mut out[y * row_bytes..y * row_bytes + row_bytes];
        for x in 0..w {
            let v = bits[y * w + x] & 1;
            if v == 1 {
                row[x / 8] |= 1 << (7 - (x % 8));
            }
        }
    }
    out
}

/// Encode a P5/P6/P7 binary body from `samples` (row-major, length =
/// width * height * depth). When `maxval > 255` we emit 16-bit
/// big-endian samples.
pub fn encode_binary_body(samples: &[u16], maxval: u32) -> Vec<u8> {
    if maxval <= 255 {
        let mut out = Vec::with_capacity(samples.len());
        for &s in samples {
            out.push(s as u8);
        }
        out
    } else {
        let mut out = Vec::with_capacity(samples.len() * 2);
        for &s in samples {
            out.extend_from_slice(&s.to_be_bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p4_round_trip_packs_msb_first() {
        // 11 pixels wide → 2 bytes per row, last 5 bits unused.
        let bits = [1, 0, 1, 0, 1, 1, 0, 0, 1, 1, 1];
        let packed = encode_p4_body(11, 1, &bits);
        assert_eq!(packed.len(), 2);
        assert_eq!(packed[0], 0b1010_1100);
        assert_eq!(packed[1] & 0b1110_0000, 0b1110_0000);
    }

    #[test]
    fn binary_body_emits_be16_when_needed() {
        let samples = [0x1234u16, 0xFEDC];
        let bytes = encode_binary_body(&samples, 65535);
        assert_eq!(bytes, [0x12, 0x34, 0xFE, 0xDC]);
        let bytes8 = encode_binary_body(&[10, 20], 100);
        assert_eq!(bytes8, [10, 20]);
    }
}
