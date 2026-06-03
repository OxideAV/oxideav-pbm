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
#[derive(Debug)]
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

    // Verify the body actually contains enough bytes for the claimed
    // dimensions BEFORE allocating the sample buffer — otherwise a
    // malformed header with `width * height` in the billions would
    // OOM the process. The exact `need` is per-magic; compute it
    // here so the allocation below is bounded by trustworthy input
    // length.
    let need = match h.magic {
        Magic::P4BinaryBitmap => w.div_ceil(8).checked_mul(hh),
        Magic::P5BinaryGraymap | Magic::P6BinaryPixmap | Magic::P7Pam => {
            let bps = h.bits_per_sample();
            let bytes_per_sample = if bps == 16 { 2 } else { 1 };
            total_samples.checked_mul(bytes_per_sample)
        }
        _ => return Err(Error::invalid("decode_binary called with non-binary magic")),
    }
    .ok_or_else(|| Error::invalid("Netpbm: dimension overflow"))?;
    if data.len() < need {
        return Err(Error::invalid("Netpbm: pixel data truncated"));
    }

    let mut out = vec![0u16; total_samples];

    match h.magic {
        Magic::P4BinaryBitmap => {
            // 1 bit per pixel, packed MSB-first, rows padded to a byte
            // boundary. Per the spec a 1-bit means BLACK.
            let row_bytes = w.div_ceil(8);
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
            if bytes_per_sample == 1 {
                for (i, b) in data[..need].iter().enumerate() {
                    out[i] = *b as u16;
                }
            } else {
                // 16-bit samples are big-endian on disk; the in-memory
                // `[u16]` is native. Walk `chunks_exact(2)` zipped with
                // `out.iter_mut()` so LLVM can lower the inner load /
                // byte-swap / store sequence to a vectorised
                // `from_be_bytes` lane (`REV16.16B` on aarch64,
                // `pshufb` / `vpshufb` on x86) instead of going via
                // indexed access. Mirrors the row-level shape used by
                // the PFM 32-bit helper in `src/pfm.rs`.
                read_be16_row(&data[..need], &mut out[..total_samples]);
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
        let mut out = vec![0u8; samples.len()];
        for (s, d) in samples.iter().zip(out.iter_mut()) {
            *d = *s as u8;
        }
        out
    } else {
        // 16-bit samples emit big-endian on disk. Pre-size the
        // destination and walk `chunks_exact_mut(2)` zipped with
        // `samples.iter()` so the inner load / byte-swap / store
        // sequence lowers to a vector `swap_bytes` lane
        // (`REV16.16B` on aarch64, `pshufb` / `vpshufb` on x86)
        // instead of running through `Vec::extend_from_slice`. Same
        // shape as the PFM 32-bit helper in `src/pfm.rs`.
        let mut out = vec![0u8; samples.len() * 2];
        write_be16_row(samples, &mut out);
        out
    }
}

/// Decode `src` as a big-endian `u16` row into `dst`. `src.len()` must
/// be `dst.len() * 2`. The inner loop walks `chunks_exact(2)` over
/// `src` zipped with `dst.iter_mut()` so LLVM can lower the load /
/// byte-swap / store sequence to a vector `from_be_bytes` lane
/// (`REV16.16B` on aarch64; `pshufb` / `vpshufb` on x86).
#[inline]
fn read_be16_row(src: &[u8], dst: &mut [u16]) {
    debug_assert_eq!(src.len(), dst.len() * 2);
    for (s, d) in src.chunks_exact(2).zip(dst.iter_mut()) {
        *d = u16::from_be_bytes([s[0], s[1]]);
    }
}

/// Encode `src` as a big-endian `u16` row into `dst`. `dst.len()` must
/// be `src.len() * 2`. The inner loop walks `src.iter()` zipped with
/// `dst.chunks_exact_mut(2)` so LLVM can lower the load / byte-swap /
/// store sequence to a vector `to_be_bytes` lane (`REV16.16B` on
/// aarch64; `pshufb` / `vpshufb` on x86).
#[inline]
fn write_be16_row(src: &[u16], dst: &mut [u8]) {
    debug_assert_eq!(dst.len(), src.len() * 2);
    for (s, d) in src.iter().zip(dst.chunks_exact_mut(2)) {
        let b = s.to_be_bytes();
        d[0] = b[0];
        d[1] = b[1];
    }
}

/// Row-level byte-swap for 2-byte samples already laid out as bytes.
/// `src` is read as little-endian `u16` samples and `dst` receives the
/// same samples big-endian (or vice versa — the swap is its own
/// inverse). `src.len()` and `dst.len()` must be equal and a multiple
/// of 2.
///
/// The encode hot paths for `Gray16Le` / `Rgb48Le` / `Rgba64Le` planes
/// take an LE-byte plane directly and write BE-byte Netpbm samples
/// without ever materialising a `Vec<u16>`. The previous implementation
/// used `for chunk in row.chunks_exact(2) { out.push(chunk[1]);
/// out.push(chunk[0]); }`, which forced the swap through individual
/// `Vec::push` calls and inhibited SIMD lowering. Walking
/// `chunks_exact(2)` zipped with `chunks_exact_mut(2)` over a
/// pre-resized `&mut [u8]` destination lets LLVM lower the inner load /
/// byte-swap / store sequence to a vector `swap_bytes` lane
/// (`REV16.16B` on aarch64; `pshufb` / `vpshufb` on x86). Same shape as
/// the PFM 32-bit helper [`crate::pfm::swap_bytes_u32_row`] introduced
/// in round 205.
#[inline]
pub(crate) fn swap_bytes_u16_row(src: &[u8], dst: &mut [u8]) {
    debug_assert_eq!(src.len(), dst.len());
    debug_assert_eq!(src.len() % 2, 0);
    for (s, d) in src.chunks_exact(2).zip(dst.chunks_exact_mut(2)) {
        d[0] = s[1];
        d[1] = s[0];
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
    fn binary_huge_dimension_does_not_oom() {
        // Regression: a P5 header that claims width * height >
        // body.len() must reject before allocating the sample buffer.
        // Pre-fix the `vec![0u16; total_samples]` allocation ran
        // ahead of the body-length check and OOMed on huge headers.
        use crate::header::{parse_header, Magic};
        let buf = b"P5\n2 200888808\n255\n\x00\x01\x02\x03";
        let h = parse_header(buf).unwrap();
        assert_eq!(h.magic, Magic::P5BinaryGraymap);
        let err = decode_binary(&h, &buf[h.data_offset..]).unwrap_err();
        match err {
            crate::error::PbmError::InvalidData(s) => {
                assert!(s.contains("truncated"), "unexpected message: {s}");
            }
            other => panic!("expected InvalidData, got {other:?}"),
        }
    }

    #[test]
    fn binary_body_emits_be16_when_needed() {
        let samples = [0x1234u16, 0xFEDC];
        let bytes = encode_binary_body(&samples, 65535);
        assert_eq!(bytes, [0x12, 0x34, 0xFE, 0xDC]);
        let bytes8 = encode_binary_body(&[10, 20], 100);
        assert_eq!(bytes8, [10, 20]);
    }

    #[test]
    fn read_be16_row_decodes_every_sample() {
        // 4 samples across 8 bytes, mixing high/low bytes to surface
        // any accidental byte ordering bug.
        let src: [u8; 8] = [0x00, 0x01, 0xFF, 0xFE, 0x12, 0x34, 0x80, 0x00];
        let mut dst = [0u16; 4];
        read_be16_row(&src, &mut dst);
        assert_eq!(dst, [0x0001, 0xFFFE, 0x1234, 0x8000]);
    }

    #[test]
    fn write_be16_row_encodes_every_sample() {
        let src = [0x0001u16, 0xFFFE, 0x1234, 0x8000];
        let mut dst = [0u8; 8];
        write_be16_row(&src, &mut dst);
        assert_eq!(dst, [0x00, 0x01, 0xFF, 0xFE, 0x12, 0x34, 0x80, 0x00]);
    }

    #[test]
    fn be16_row_helpers_round_trip() {
        // Self-inverse: encode-then-decode reconstructs the original
        // sample sequence exactly, with no boundary corruption.
        let src: [u16; 7] = [0x0000, 0x00FF, 0xFF00, 0xFFFF, 0xDEAD, 0xBEEF, 0xCAFE];
        let mut bytes = vec![0u8; src.len() * 2];
        write_be16_row(&src, &mut bytes);
        let mut round_trip = vec![0u16; src.len()];
        read_be16_row(&bytes, &mut round_trip);
        assert_eq!(round_trip.as_slice(), &src);
    }

    #[test]
    fn swap_bytes_u16_row_swaps_every_sample() {
        // Six samples mixing high/low bytes to surface any indexing
        // off-by-one in the chunked swap kernel.
        let src: [u8; 12] = [
            0x12, 0x34, // sample 0
            0xff, 0x00, // sample 1
            0xa5, 0x5a, // sample 2
            0x00, 0xff, // sample 3
            0xde, 0xad, // sample 4
            0xbe, 0xef, // sample 5
        ];
        let mut dst = [0u8; 12];
        swap_bytes_u16_row(&src, &mut dst);
        assert_eq!(
            dst,
            [
                0x34, 0x12, // sample 0 reversed
                0x00, 0xff, // sample 1 reversed
                0x5a, 0xa5, // sample 2 reversed
                0xff, 0x00, // sample 3 reversed
                0xad, 0xde, // sample 4 reversed
                0xef, 0xbe, // sample 5 reversed
            ]
        );
    }

    #[test]
    fn swap_bytes_u16_row_is_self_inverse() {
        // Swapping twice must reconstruct the original byte sequence.
        let src: [u8; 10] = [0xaa, 0xbb, 0x01, 0x02, 0xfe, 0xed, 0x80, 0x00, 0xca, 0xfe];
        let mut once = [0u8; 10];
        swap_bytes_u16_row(&src, &mut once);
        let mut twice = [0u8; 10];
        swap_bytes_u16_row(&once, &mut twice);
        assert_eq!(twice, src);
    }

    #[test]
    fn swap_bytes_u16_row_matches_per_sample_le_to_be() {
        // The helper's output must agree byte-for-byte with the scalar
        // `u16::from_le_bytes(…).to_be_bytes()` reference path the
        // encoder hot paths used before the round-217 refactor.
        let src: [u8; 8] = [0x34, 0x12, 0x78, 0x56, 0xff, 0x00, 0x00, 0x80];
        let mut got = [0u8; 8];
        swap_bytes_u16_row(&src, &mut got);
        let mut expected = [0u8; 8];
        for (s, d) in src.chunks_exact(2).zip(expected.chunks_exact_mut(2)) {
            let v = u16::from_le_bytes([s[0], s[1]]).to_be_bytes();
            d.copy_from_slice(&v);
        }
        assert_eq!(got, expected);
    }
}
