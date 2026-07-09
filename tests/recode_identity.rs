//! Decode → encode → decode **fixed-point** identity.
//!
//! The per-format round-trip tests in `encode_roundtrip.rs` build a
//! synthetic image and check that a single encode/decode pass preserves
//! the plane. This suite pins the stronger *fixed-point* contract the
//! `recode` fuzz target enforces on arbitrary inputs:
//!
//!   for every image the decoder can PRODUCE,
//!     `decode(encode(img))` reproduces `img` byte-for-byte —
//!     same width, height, pixel format, plane stride and plane data.
//!
//! Why it must hold: the decoder always emits a tightly-packed plane
//! (stride == the format's minimum row bytes) at the format's natural
//! `MAXVAL` (255 / 65535 / IEEE-754). The encoder picks the on-disk
//! magic that carries exactly that layout, so a second decode has to
//! land back on the identical image. Any drift here is an encoder /
//! decoder asymmetry bug (e.g. a channel-order swap, a wrong maxval, a
//! byte-order flip, or a stride miscalculation) that the single-pass
//! tests could miss if both directions shared the same mistake.
//!
//! `Bgra` is intentionally absent: it is an encode-side *input* format
//! (the decoder never produces it — a BGRA source is emitted as PAM
//! `RGB_ALPHA` and decodes back as `Rgba`), so it is not part of the
//! decoder's output set and cannot be a fixed point. That direction is
//! covered separately below.

use oxideav_pbm::{decode_pbm, encode_pbm, PbmImage, PbmPixelFormat, PbmPlane};

/// Minimum (tight) row stride the decoder emits for `(format, width)`.
fn tight_stride(format: PbmPixelFormat, width: usize) -> usize {
    match format {
        PbmPixelFormat::MonoBlack => width.div_ceil(8),
        PbmPixelFormat::Gray8 => width,
        PbmPixelFormat::Gray16Le | PbmPixelFormat::Ya8 => width * 2,
        PbmPixelFormat::Rgb24 => width * 3,
        PbmPixelFormat::Ya16Le | PbmPixelFormat::GrayF32 => width * 4,
        PbmPixelFormat::Rgb48Le => width * 6,
        PbmPixelFormat::Rgba | PbmPixelFormat::Bgra => width * 4,
        PbmPixelFormat::Rgba64Le => width * 8,
        PbmPixelFormat::RgbF32 => width * 12,
    }
}

/// Build a decoder-shaped image: tight stride, deterministic bytes.
fn make_image(format: PbmPixelFormat, w: u32, h: u32) -> PbmImage {
    let stride = tight_stride(format, w as usize);
    let mut data = vec![0u8; stride * h as usize];
    // Fill with a reproducible xorshift byte stream so every channel /
    // byte lane carries a distinct value (catches channel-order and
    // byte-order swaps that a uniform fill would hide).
    let mut state: u32 = 0x9E37_79B9 ^ (w.wrapping_mul(2_654_435_761)).wrapping_add(h);
    for b in data.iter_mut() {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        *b = (state & 0xFF) as u8;
    }
    // For the two float formats a raw xorshift byte can encode a NaN /
    // subnormal; those still round-trip bit-exactly (PFM samples are
    // copied verbatim), but keep the payload finite so the assertion is
    // meaningful rather than accidentally comparing NaN payloads.
    if matches!(format, PbmPixelFormat::GrayF32 | PbmPixelFormat::RgbF32) {
        for (i, chunk) in data.chunks_exact_mut(4).enumerate() {
            let v = (i as f32) * 0.5 - 3.0;
            chunk.copy_from_slice(&v.to_le_bytes());
        }
    }
    // The decoder emits `MonoBlack` planes with the sub-byte row padding
    // zeroed (bits past `x == width - 1` in each row's last byte). A
    // directly-constructed image is only a valid *decoder output* — and
    // therefore a candidate fixed point — if its pad bits are already
    // zero, so mask them here. (The dirty-pad-bit behaviour is asserted
    // separately in `monoblack_padding_bits_survive_fixed_point`.)
    if format == PbmPixelFormat::MonoBlack && w % 8 != 0 {
        let keep = 0xFFu8 << (8 - (w % 8)); // top `w % 8` bits
        for row in data.chunks_exact_mut(stride) {
            if let Some(last) = row.last_mut() {
                *last &= keep;
            }
        }
    }
    PbmImage {
        width: w,
        height: h,
        pixel_format: format,
        planes: vec![PbmPlane { stride, data }],
        pts: None,
    }
}

fn assert_fixed_point(format: PbmPixelFormat, w: u32, h: u32) {
    let img = make_image(format, w, h);
    let bytes = encode_pbm(&img).unwrap_or_else(|e| panic!("encode {format:?} {w}x{h}: {e}"));
    let (back, fmt) =
        decode_pbm(&bytes).unwrap_or_else(|e| panic!("re-decode {format:?} {w}x{h}: {e}"));

    assert_eq!(fmt, format, "pixel format drifted for {format:?} {w}x{h}");
    assert_eq!(back.width, img.width, "width drifted for {format:?}");
    assert_eq!(back.height, img.height, "height drifted for {format:?}");
    assert_eq!(
        back.pixel_format, img.pixel_format,
        "image format drifted for {format:?}"
    );
    assert_eq!(
        back.planes.len(),
        1,
        "expected one plane for {format:?} {w}x{h}"
    );
    assert_eq!(
        back.planes[0].stride, img.planes[0].stride,
        "stride drifted for {format:?} {w}x{h}"
    );
    assert_eq!(
        back.planes[0].data, img.planes[0].data,
        "plane data drifted for {format:?} {w}x{h}"
    );

    // Second pass must be a genuine fixed point, not merely stable-ish:
    // re-encoding the re-decoded image yields byte-identical output.
    let bytes2 = encode_pbm(&back).unwrap();
    assert_eq!(
        bytes, bytes2,
        "encode not idempotent for {format:?} {w}x{h}"
    );
}

const DECODE_PRODUCIBLE: &[PbmPixelFormat] = &[
    PbmPixelFormat::MonoBlack,
    PbmPixelFormat::Gray8,
    PbmPixelFormat::Gray16Le,
    PbmPixelFormat::Rgb24,
    PbmPixelFormat::Rgb48Le,
    PbmPixelFormat::Rgba,
    PbmPixelFormat::Rgba64Le,
    PbmPixelFormat::Ya8,
    PbmPixelFormat::Ya16Le,
    PbmPixelFormat::GrayF32,
    PbmPixelFormat::RgbF32,
];

#[test]
fn every_decode_producible_format_is_a_fixed_point() {
    for &format in DECODE_PRODUCIBLE {
        // Several dimensions, including widths that are not multiples of
        // 8 (the MonoBlack byte-packing edge) and a 1x1 minimum.
        for &(w, h) in &[(1, 1), (7, 3), (8, 8), (13, 5), (16, 12), (17, 2)] {
            assert_fixed_point(format, w, h);
        }
    }
}

#[test]
fn monoblack_padding_bits_survive_fixed_point() {
    // A width of 11 leaves 5 pad bits in the second byte of every row.
    // The decoder must zero them and the encoder must not read them back
    // as pixels, so the fixed point holds regardless of what the
    // (constructed) pad bits were.
    let mut img = make_image(PbmPixelFormat::MonoBlack, 11, 4);
    // Dirty the pad bits deliberately: set every trailing bit in each
    // row's last byte. tight_stride(11) == 2, pixels occupy bits 0..11.
    for row in img.planes[0].data.chunks_exact_mut(2) {
        row[1] |= 0b0001_1111; // bits for x=11..15 (pad region)
    }
    let bytes = encode_pbm(&img).unwrap();
    let (back, _) = decode_pbm(&bytes).unwrap();
    // The decoded image zeros pad bits; re-encoding it is the true fixed
    // point. Confirm a second recode is stable.
    let bytes2 = encode_pbm(&back).unwrap();
    let (back2, _) = decode_pbm(&bytes2).unwrap();
    assert_eq!(back.planes[0].data, back2.planes[0].data);
    assert_eq!(bytes, bytes2);
    // And the visible pixels (bits 0..11 of each row) are preserved.
    for (orig, rt) in img.planes[0]
        .data
        .chunks_exact(2)
        .zip(back.planes[0].data.chunks_exact(2))
    {
        assert_eq!(orig[0], rt[0], "first byte (8 pixels) must match");
        // High 3 bits of the second byte are pixels x=8,9,10.
        assert_eq!(orig[1] & 0b1110_0000, rt[1] & 0b1110_0000);
        // Pad bits must be zeroed by the decoder.
        assert_eq!(rt[1] & 0b0001_1111, 0, "pad bits must be zero after decode");
    }
}

#[test]
fn bgra_input_decodes_back_as_channel_swapped_rgba() {
    // Bgra is an encode-input-only format: the decoder never produces it.
    // Encoding a BGRA image emits PAM RGB_ALPHA (channels reordered to
    // R,G,B,A), so a decode lands on Rgba with the channels swapped back
    // into RGBA order. This documents the one format that is *not* a
    // fixed point and pins the exact swap.
    let (w, h) = (3u32, 2u32);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for i in 0..(w * h) {
        let b = (i * 10) as u8;
        let g = (i * 10 + 1) as u8;
        let r = (i * 10 + 2) as u8;
        let a = (i * 10 + 3) as u8;
        data.extend_from_slice(&[b, g, r, a]); // BGRA on disk in-memory
    }
    let img = PbmImage {
        width: w,
        height: h,
        pixel_format: PbmPixelFormat::Bgra,
        planes: vec![PbmPlane {
            stride: (w * 4) as usize,
            data: data.clone(),
        }],
        pts: None,
    };
    let bytes = encode_pbm(&img).unwrap();
    let (back, fmt) = decode_pbm(&bytes).unwrap();
    assert_eq!(fmt, PbmPixelFormat::Rgba);
    // Each source BGRA quad must reappear as RGBA.
    for (src, rt) in data
        .chunks_exact(4)
        .zip(back.planes[0].data.chunks_exact(4))
    {
        let (b, g, r, a) = (src[0], src[1], src[2], src[3]);
        assert_eq!(rt, &[r, g, b, a]);
    }
}
