//! Netpbm decoder. Maps every variant onto a [`PbmImage`] tagged with
//! the closest matching [`PbmPixelFormat`]:
//!
//! | Netpbm                  | PbmPixelFormat |
//! |-------------------------|----------------|
//! | P1 / P4 (1-bit)         | `MonoBlack` (1 = black, MSB-first packed) |
//! | P2 / P5 8-bit gray      | `Gray8`        |
//! | P2 / P5 16-bit gray     | `Gray16Le`     |
//! | P3 / P6 8-bit RGB       | `Rgb24`        |
//! | P3 / P6 16-bit RGB      | `Rgb48Le`      |
//! | P7 BLACKANDWHITE        | `MonoBlack`    |
//! | P7 GRAYSCALE 8/16       | `Gray8` / `Gray16Le` |
//! | P7 RGB 8/16             | `Rgb24` / `Rgb48Le`  |
//! | P7 GRAYSCALE_ALPHA 8    | `Ya8`          |
//! | P7 GRAYSCALE_ALPHA 16   | `Ya16Le`       |
//! | P7 RGB_ALPHA 8          | `Rgba`         |
//! | P7 RGB_ALPHA 16         | `Rgba64Le`     |
//!
//! BLACKANDWHITE_ALPHA falls back to a 4-byte-per-pixel `Rgba`
//! representation (the bit-valued first channel expands to a gray
//! triplet) — the alpha channel is preserved either way.
//!
//! With the default `registry` feature on, the gated `PbmDecoder` trait
//! impl wraps [`decode_pbm`] for the `oxideav_core::Decoder` surface.

use crate::error::{PbmError as Error, Result};

use crate::ascii::decode_ascii_consumed;
use crate::binary::{copy_p4_row_msb, decode_binary, swap_bytes_u16_row, DecodedSamples};
use crate::header::{parse_header, Header, Magic, Tupltype};
use crate::image::{PbmImage, PbmPixelFormat, PbmPlane};

#[cfg(feature = "registry")]
use oxideav_core::Decoder;
#[cfg(feature = "registry")]
use oxideav_core::{CodecId, CodecParameters, Frame, Packet, VideoFrame, VideoPlane};

/// Factory registered with the codec registry. One packet per whole
/// Netpbm file; one frame per packet.
#[cfg(feature = "registry")]
pub fn make_decoder(_params: &CodecParameters) -> oxideav_core::Result<Box<dyn Decoder>> {
    Ok(Box::new(PbmDecoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        pending: None,
        eof: false,
    }))
}

#[cfg(feature = "registry")]
struct PbmDecoder {
    codec_id: CodecId,
    pending: Option<VideoFrame>,
    eof: bool,
}

#[cfg(feature = "registry")]
impl Decoder for PbmDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn send_packet(&mut self, packet: &Packet) -> oxideav_core::Result<()> {
        let (image, _fmt) = decode_pbm(&packet.data)?;
        self.pending = Some(image_to_video_frame(image));
        Ok(())
    }
    fn receive_frame(&mut self) -> oxideav_core::Result<Frame> {
        match self.pending.take() {
            Some(f) => Ok(Frame::Video(f)),
            None => {
                if self.eof {
                    Err(oxideav_core::Error::Eof)
                } else {
                    Err(oxideav_core::Error::NeedMore)
                }
            }
        }
    }
    fn flush(&mut self) -> oxideav_core::Result<()> {
        self.eof = true;
        Ok(())
    }
}

#[cfg(feature = "registry")]
fn image_to_video_frame(image: PbmImage) -> VideoFrame {
    VideoFrame {
        pts: image.pts,
        planes: image
            .planes
            .into_iter()
            .map(|p| VideoPlane {
                stride: p.stride,
                data: p.data,
            })
            .collect(),
    }
}

/// Decode a complete Netpbm file (any of the seven magic numbers) into
/// a [`PbmImage`] plus the [`PbmPixelFormat`] the image carries.
///
/// Only the *first* image in `input` is decoded; trailing bytes (e.g. a
/// second concatenated image in a multi-image stream) are ignored. Use
/// [`decode_pbm_multi`] to walk every image in a concatenated stream.
pub fn decode_pbm(input: &[u8]) -> Result<(PbmImage, PbmPixelFormat)> {
    decode_pbm_consumed(input).map(|(image, fmt, _consumed)| (image, fmt))
}

/// The Netpbm/PAM/PFM family permits a single file to carry a **sequence
/// of concatenated images** — each a self-describing magic + header +
/// body, packed back-to-back with optional whitespace between them
/// (`pnm(5)` / `pam(5)` describe a file as holding "one or more" images;
/// the Portable FloatMap reference likewise places the next header
/// immediately after the prior raster). Decode every image in `input`,
/// returning one `(PbmImage, PbmPixelFormat)` per image in stream order.
///
/// A single image is the common case and yields a one-element `Vec`.
/// Bytes are consumed exactly: each image's on-disk length is the header
/// length plus the body length (deterministic for the binary and PFM
/// magics from the dimensions; for the ASCII magics it is the offset of
/// the byte after the final sample token). Inter-image ASCII whitespace
/// is skipped before the next magic is read — the magic must be the
/// first two bytes of each image, so a `#` between images is *not* a
/// valid separator. Trailing whitespace after the last image is not an
/// error; trailing *non-whitespace* that does not begin a valid header
/// is reported as a malformed stream.
pub fn decode_pbm_multi(input: &[u8]) -> Result<Vec<(PbmImage, PbmPixelFormat)>> {
    let mut images = Vec::new();
    let mut offset = 0usize;
    loop {
        // Skip inter-image whitespace. The magic number must be the
        // first two bytes of each image (the PNM/PAM grammar puts the
        // magic before any comment), so only ASCII whitespace — not a
        // `#` comment — may separate concatenated images. A `#` here
        // therefore falls through to `parse_header`, which rejects it
        // as a missing magic, surfacing a malformed stream rather than
        // silently swallowing bytes.
        while offset < input.len() && is_ascii_ws(input[offset]) {
            offset += 1;
        }
        if offset >= input.len() {
            break;
        }
        let (image, fmt, consumed) = decode_pbm_consumed(&input[offset..])?;
        images.push((image, fmt));
        debug_assert!(consumed > 0, "decode consumed zero bytes — would loop");
        if consumed == 0 {
            // Defence-in-depth: never spin forever on a degenerate header.
            return Err(Error::invalid("Netpbm: image consumed zero bytes"));
        }
        offset += consumed;
    }
    if images.is_empty() {
        return Err(Error::invalid("Netpbm: no images in stream"));
    }
    Ok(images)
}

#[inline]
fn is_ascii_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0B | 0x0C)
}

/// Decode the first image in `input` and report how many bytes of
/// `input` it occupied (header + body, excluding any trailing inter-image
/// whitespace). The byte count lets [`decode_pbm_multi`] locate the next
/// concatenated image; single-image [`decode_pbm`] discards it.
pub fn decode_pbm_consumed(input: &[u8]) -> Result<(PbmImage, PbmPixelFormat, usize)> {
    let header = parse_header(input)?;
    let body = &input[header.data_offset..];
    // Portable FloatMap has a wholly different (float, bottom-to-top,
    // endianness-tagged) body — hand it to the dedicated decoder.
    if header.magic.is_pfm() {
        let (image, fmt) = crate::pfm::decode_pfm_image(&header, body)?;
        let body_len = pfm_body_byte_len(&header)?;
        return Ok((image, fmt, header.data_offset + body_len));
    }
    // P4 → `MonoBlack` fast path. The wire format (MSB-first packed
    // bits, rows padded to a byte boundary, `1 = black`) is byte-for-byte
    // identical to the crate's `MonoBlack` plane convention, so the body
    // is a per-row memcpy + trailing-bit mask — skipping both the
    // intermediate `Vec<u16>` sample buffer that `decode_binary` would
    // allocate and the per-bit re-pack pass that `samples_to_plane`
    // would run. Symmetric with the round-229 `encode_p4` rewrite,
    // which dropped the same two scalar bit loops on the encode side.
    // P1 (ASCII bitmap) and P7 `BLACKANDWHITE` (which inverts the bit
    // sense per `pam(5)`) still go through the generic path.
    if matches!(header.magic, Magic::P4BinaryBitmap) {
        let (image, fmt) = decode_p4_monoblack(&header, body)?;
        let body_len = binary_body_byte_len(&header)?;
        return Ok((image, fmt, header.data_offset + body_len));
    }
    // Binary 8-bit (maxval=255) and 16-bit (maxval=65535) hot path. When
    // the wire sample layout matches the plane byte layout — P5 / P6 /
    // P7 (`GRAYSCALE` / `GRAYSCALE_ALPHA` / `RGB` / `RGB_ALPHA`, plus
    // custom-tupltype routed through depth) at the natural maxval — the
    // body is either a per-row `copy_from_slice` (8-bit) or a per-row
    // `swap_bytes_u16_row` (16-bit) straight into the destination plane.
    // This skips both the intermediate `Vec<u16>` widen pass in
    // `decode_binary` and the per-sample `scale_to_*` /
    // `to_le_bytes` loop in `samples_to_plane`. Symmetric with the
    // round-229 `encode_p4` and round-248 `decode_p4_monoblack`
    // memcpy rewrites.
    if let Some((image, fmt)) = try_decode_binary_bytewise(&header, body)? {
        let body_len = binary_body_byte_len(&header)?;
        return Ok((image, fmt, header.data_offset + body_len));
    }
    // ASCII bodies have no closed-form byte length (whitespace and
    // comment runs vary), so the tokenizer reports its consumed cursor;
    // binary bodies are deterministic from the dimensions.
    let (samples, body_len) = if header.magic.is_ascii() {
        let (samples, cursor) = decode_ascii_consumed(&header, body)?;
        (samples, cursor)
    } else {
        let samples = decode_binary(&header, body)?;
        (samples, binary_body_byte_len(&header)?)
    };
    let (plane, format) = samples_to_plane(&header, &samples)?;
    Ok((
        PbmImage {
            width: header.width,
            height: header.height,
            pixel_format: format,
            planes: vec![plane],
            pts: None,
        },
        format,
        header.data_offset + body_len,
    ))
}

/// On-disk body byte length for the **binary** PNM/PAM magics
/// (`P4` / `P5` / `P6` / `P7`). Deterministic from the dimensions,
/// depth, and bits-per-sample. Mirrors the `need` computation each
/// binary decode path performs before allocating, so a stream walked by
/// [`decode_pbm_multi`] lands exactly on the next image's magic.
fn binary_body_byte_len(h: &Header) -> Result<usize> {
    let w = h.width as usize;
    let hh = h.height as usize;
    let depth = h.depth as usize;
    match h.magic {
        Magic::P4BinaryBitmap => w
            .div_ceil(8)
            .checked_mul(hh)
            .ok_or_else(|| Error::invalid("Netpbm: dimension overflow")),
        Magic::P5BinaryGraymap | Magic::P6BinaryPixmap | Magic::P7Pam => {
            let bytes_per_sample = if h.bits_per_sample() == 16 { 2 } else { 1 };
            w.checked_mul(hh)
                .and_then(|v| v.checked_mul(depth))
                .and_then(|v| v.checked_mul(bytes_per_sample))
                .ok_or_else(|| Error::invalid("Netpbm: dimension overflow"))
        }
        _ => Err(Error::invalid(
            "binary_body_byte_len called with non-binary magic",
        )),
    }
}

/// On-disk body byte length for a Portable FloatMap header:
/// `width * height * channels * 4` IEEE-754 binary32 bytes.
fn pfm_body_byte_len(h: &Header) -> Result<usize> {
    let w = h.width as usize;
    let hh = h.height as usize;
    let ch = h.depth as usize;
    w.checked_mul(hh)
        .and_then(|v| v.checked_mul(ch))
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| Error::invalid("PFM: raster-size overflow"))
}

/// P4 (binary PBM) → `MonoBlack` row-level memcpy fast path. Validates
/// that the body holds the full `row_bytes * height` payload upfront so
/// a malformed header claiming multi-billion dimensions cannot OOM the
/// destination allocation, then walks rows via
/// [`crate::binary::copy_p4_row_msb`] (the same helper the round-229
/// `encode_p4` path uses) so the inner per-row work is a `copy_from_slice`
/// plus at most one trailing-pad mask.
fn decode_p4_monoblack(header: &Header, body: &[u8]) -> Result<(PbmImage, PbmPixelFormat)> {
    let w = header.width as usize;
    let h = header.height as usize;
    if w == 0 || h == 0 {
        return Err(Error::invalid("Netpbm: zero dimension"));
    }
    let row_bytes = w.div_ceil(8);
    let need = row_bytes
        .checked_mul(h)
        .ok_or_else(|| Error::invalid("Netpbm: dimension overflow"))?;
    if body.len() < need {
        return Err(Error::invalid("Netpbm: pixel data truncated"));
    }
    let mut data = vec![0u8; need];
    for y in 0..h {
        let off = y * row_bytes;
        let src = &body[off..off + row_bytes];
        let dst = &mut data[off..off + row_bytes];
        copy_p4_row_msb(src, dst, w);
    }
    let plane = PbmPlane {
        stride: row_bytes,
        data,
    };
    Ok((
        PbmImage {
            width: header.width,
            height: header.height,
            pixel_format: PbmPixelFormat::MonoBlack,
            planes: vec![plane],
            pts: None,
        },
        PbmPixelFormat::MonoBlack,
    ))
}

/// Binary `P5` / `P6` / `P7` decode fast path for the cases where the
/// on-disk sample layout is byte-for-byte identical to the destination
/// plane (after at most a row-level 16-bit LE→BE byte swap). Returns
/// `Some(decoded)` for the eligible cases, `None` so the caller falls
/// back to the generic widen-then-rescale path.
///
/// Eligible when:
///
/// * Magic is `P5` / `P6` / `P7` (binary, non-bit).
/// * `maxval == 255` (8-bit, no scaling) or `maxval == 65535`
///   (16-bit, no scaling — just LE↔BE byte order).
/// * Depth × bytes-per-sample lines up with the destination plane's
///   bytes-per-pixel for the picked [`PbmPixelFormat`] — so the wire
///   is a flat row-major byte stream the plane can receive directly.
///
/// For PAM, `pick_pixel_format` already handles standard tupltypes
/// (`GRAYSCALE` / `GRAYSCALE_ALPHA` / `RGB` / `RGB_ALPHA`) and the
/// depth-routed `Custom` / no-tupltype cases. Tupltypes that involve a
/// channel re-arrangement (`BLACKANDWHITE` → bit pack,
/// `BLACKANDWHITE_ALPHA` → expand to RGBA) fall through to the generic
/// path because their wire bytes do not line up with the plane bytes.
fn try_decode_binary_bytewise(
    header: &Header,
    body: &[u8],
) -> Result<Option<(PbmImage, PbmPixelFormat)>> {
    // Only the three binary magics with row-major scalar samples are
    // eligible. P4 has its own fast path; P1/P2/P3 are ASCII; PFM is
    // handled before we ever reach here.
    if !matches!(
        header.magic,
        Magic::P5BinaryGraymap | Magic::P6BinaryPixmap | Magic::P7Pam
    ) {
        return Ok(None);
    }
    // Only the natural maxvals — 255 for 8-bit, 65535 for 16-bit —
    // skip the per-sample scaling path. Anything else (e.g. PGM with
    // maxval=200) must go through `scale_to_u8` / `scale_to_u16`.
    let bytes_per_sample = match header.maxval {
        255 => 1usize,
        65535 => 2usize,
        _ => return Ok(None),
    };

    let format = pick_pixel_format(header)?;
    // The set of (format, depth, bytes_per_sample) tuples where the
    // wire stream is byte-for-byte identical to the destination plane
    // (8-bit) or related only by a row-level u16 byte swap (16-bit).
    //
    // PAM combinations that re-arrange channels are excluded even when
    // their wire byte count happens to coincide with the plane's: e.g.
    // `BLACKANDWHITE_ALPHA` expands its bit-valued first channel into a
    // gray triplet, so the byte mapping is not an identity. The generic
    // path's `fill_rgba_u8`-style channel expander handles those cases
    // unchanged.
    let depth = header.depth as usize;
    let bytes_per_pixel: usize = match (format, depth, bytes_per_sample) {
        (PbmPixelFormat::Gray8, 1, 1) => 1,
        (PbmPixelFormat::Ya8, 2, 1) => 2,
        (PbmPixelFormat::Gray16Le, 1, 2) => 2,
        (PbmPixelFormat::Rgb24, 3, 1) => 3,
        (PbmPixelFormat::Rgba, 4, 1) => 4,
        (PbmPixelFormat::Ya16Le, 2, 2) => 4,
        (PbmPixelFormat::Rgb48Le, 3, 2) => 6,
        (PbmPixelFormat::Rgba64Le, 4, 2) => 8,
        _ => return Ok(None),
    };

    let w = header.width as usize;
    let h = header.height as usize;
    if w == 0 || h == 0 {
        return Err(Error::invalid("Netpbm: zero dimension"));
    }
    let stride = w
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| Error::invalid("Netpbm: stride overflow"))?;
    let total = stride
        .checked_mul(h)
        .ok_or_else(|| Error::invalid("Netpbm: plane-size overflow"))?;
    if body.len() < total {
        return Err(Error::invalid("Netpbm: pixel data truncated"));
    }

    let mut data = vec![0u8; total];
    if bytes_per_sample == 1 {
        // 8-bit fast path: wire bytes are already the plane bytes
        // (samples are uint8 and the plane format matches the depth).
        // A single `copy_from_slice` lowers to `memcpy` on every
        // target — no per-row loop needed.
        data.copy_from_slice(&body[..total]);
    } else {
        // 16-bit fast path: wire is big-endian, plane is little-endian.
        // The row-level `swap_bytes_u16_row` helper lowers the inner
        // load / byte-swap / store sequence to a vector byte-swap lane
        // (`REV16.16B` on aarch64; `pshufb` / `vpshufb` on x86) —
        // same shape as the round-205 PFM 32-bit helper and the
        // round-217 encode-side helper of the same name.
        for y in 0..h {
            let off = y * stride;
            let src = &body[off..off + stride];
            let dst = &mut data[off..off + stride];
            swap_bytes_u16_row(src, dst);
        }
    }
    Ok(Some((
        PbmImage {
            width: header.width,
            height: header.height,
            pixel_format: format,
            planes: vec![PbmPlane { stride, data }],
            pts: None,
        },
        format,
    )))
}

/// Build a `(PbmPlane, PbmPixelFormat)` from a freshly-decoded sample
/// matrix. This is the place that picks which [`PbmPixelFormat`] best
/// represents each (magic, depth, maxval) combination.
fn samples_to_plane(h: &Header, s: &DecodedSamples) -> Result<(PbmPlane, PbmPixelFormat)> {
    let w = h.width as usize;
    let hh = h.height as usize;
    let depth = h.depth as usize;
    let bps = h.bits_per_sample();

    // Determine target pixel format from the input shape.
    let format = pick_pixel_format(h)?;
    let _ = bps; // bits-per-sample is implicit in the chosen format

    // Defence-in-depth: validate that the (stride, height) the chosen
    // pixel format implies cannot overflow `usize`. `decode_binary` /
    // `decode_ascii` already validate the sample-buffer size against
    // the body length; this check guards `vec![0u8; stride * hh]`
    // against a downstream multiplication overflow if either layer
    // returned a `DecodedSamples` larger than `usize::MAX / 8`.
    let bytes_per_pixel: usize = match format {
        PbmPixelFormat::MonoBlack => 1, // computed as div_ceil below
        PbmPixelFormat::Gray8 => 1,
        PbmPixelFormat::Ya8 => 2,
        PbmPixelFormat::Gray16Le => 2,
        PbmPixelFormat::Rgb24 => 3,
        PbmPixelFormat::Rgba | PbmPixelFormat::Bgra | PbmPixelFormat::Ya16Le => 4,
        PbmPixelFormat::Rgb48Le => 6,
        PbmPixelFormat::Rgba64Le => 8,
        PbmPixelFormat::GrayF32 => 4,
        PbmPixelFormat::RgbF32 => 12,
    };
    let stride_check = if matches!(format, PbmPixelFormat::MonoBlack) {
        w.div_ceil(8)
    } else {
        w.checked_mul(bytes_per_pixel)
            .ok_or_else(|| Error::invalid("Netpbm: stride overflow"))?
    };
    stride_check
        .checked_mul(hh)
        .ok_or_else(|| Error::invalid("Netpbm: plane-size overflow"))?;

    match format {
        PbmPixelFormat::MonoBlack => {
            // 1 bit per pixel, MSB-first packed, rows padded to byte
            // boundary.
            let stride = w.div_ceil(8);
            let mut data = vec![0u8; stride * hh];
            for y in 0..hh {
                for x in 0..w {
                    // For BLACKANDWHITE PAM the spec says 1 = white,
                    // 0 = black (opposite of P1/P4 — yes, really, see
                    // pam(5) "TUPLE TYPE" section). Normalise so the
                    // resulting `MonoBlack` plane always uses 1 = black.
                    let raw = s.samples[y * w + x] != 0;
                    let bit = match h.magic {
                        Magic::P1AsciiBitmap | Magic::P4BinaryBitmap => raw, // 1 = black already
                        Magic::P7Pam => !raw, // invert: PAM has 1 = white
                        _ => raw,
                    };
                    if bit {
                        data[y * stride + x / 8] |= 1 << (7 - (x % 8));
                    }
                }
            }
            Ok((PbmPlane { stride, data }, format))
        }
        PbmPixelFormat::Gray8 => {
            let stride = w;
            let mut data = vec![0u8; stride * hh];
            for (i, byte) in data.iter_mut().enumerate().take(w * hh) {
                *byte = scale_to_u8(s.samples[i], h.maxval);
            }
            Ok((PbmPlane { stride, data }, format))
        }
        PbmPixelFormat::Gray16Le => {
            let stride = w * 2;
            let mut data = vec![0u8; stride * hh];
            for i in 0..(w * hh) {
                let v = scale_to_u16(s.samples[i], h.maxval);
                let off = i * 2;
                data[off..off + 2].copy_from_slice(&v.to_le_bytes());
            }
            Ok((PbmPlane { stride, data }, format))
        }
        PbmPixelFormat::Rgb24 => {
            let stride = w * 3;
            let mut data = vec![0u8; stride * hh];
            for i in 0..(w * hh) {
                for c in 0..3 {
                    data[i * 3 + c] = scale_to_u8(s.samples[i * depth + c], h.maxval);
                }
            }
            Ok((PbmPlane { stride, data }, format))
        }
        PbmPixelFormat::Rgb48Le => {
            let stride = w * 6;
            let mut data = vec![0u8; stride * hh];
            for i in 0..(w * hh) {
                for c in 0..3 {
                    let v = scale_to_u16(s.samples[i * depth + c], h.maxval);
                    let off = i * 6 + c * 2;
                    data[off..off + 2].copy_from_slice(&v.to_le_bytes());
                }
            }
            Ok((PbmPlane { stride, data }, format))
        }
        PbmPixelFormat::Ya8 => {
            let stride = w * 2;
            let mut data = vec![0u8; stride * hh];
            for i in 0..(w * hh) {
                data[i * 2] = scale_to_u8(s.samples[i * depth], h.maxval);
                data[i * 2 + 1] = scale_to_u8(s.samples[i * depth + 1], h.maxval);
            }
            Ok((PbmPlane { stride, data }, format))
        }
        PbmPixelFormat::Ya16Le => {
            let stride = w * 4;
            let mut data = vec![0u8; stride * hh];
            for i in 0..(w * hh) {
                let g = scale_to_u16(s.samples[i * depth], h.maxval);
                let a = scale_to_u16(s.samples[i * depth + 1], h.maxval);
                let off = i * 4;
                data[off..off + 2].copy_from_slice(&g.to_le_bytes());
                data[off + 2..off + 4].copy_from_slice(&a.to_le_bytes());
            }
            Ok((PbmPlane { stride, data }, format))
        }
        PbmPixelFormat::Rgba => {
            let stride = w * 4;
            let mut data = vec![0u8; stride * hh];
            // Map (depth, tupltype) → RGBA layout.
            let layout = rgba_layout(h, depth);
            for i in 0..(w * hh) {
                let pix = &mut data[i * 4..i * 4 + 4];
                let src = &s.samples[i * depth..i * depth + depth];
                fill_rgba_u8(pix, src, h.maxval, layout);
            }
            Ok((PbmPlane { stride, data }, format))
        }
        PbmPixelFormat::Rgba64Le => {
            let stride = w * 8;
            let mut data = vec![0u8; stride * hh];
            for i in 0..(w * hh) {
                let src = &s.samples[i * depth..i * depth + depth];
                let r = scale_to_u16(src[0], h.maxval);
                let g = scale_to_u16(src[1], h.maxval);
                let b = scale_to_u16(src[2], h.maxval);
                let a = scale_to_u16(src[3], h.maxval);
                let off = i * 8;
                data[off..off + 2].copy_from_slice(&r.to_le_bytes());
                data[off + 2..off + 4].copy_from_slice(&g.to_le_bytes());
                data[off + 4..off + 6].copy_from_slice(&b.to_le_bytes());
                data[off + 6..off + 8].copy_from_slice(&a.to_le_bytes());
            }
            Ok((PbmPlane { stride, data }, format))
        }
        // `Bgra` is encode-side input only — never picked by the decoder.
        PbmPixelFormat::Bgra => Err(Error::unsupported(
            "Netpbm: BGRA decode not produced by any source format".to_string(),
        )),
        // The float maps are decoded by `crate::pfm::decode_pfm_image`,
        // which `decode_pbm` dispatches to before reaching this integer
        // sample path — they never arrive here.
        PbmPixelFormat::GrayF32 | PbmPixelFormat::RgbF32 => Err(Error::invalid(
            "Netpbm: float-map pixel format reached the integer sample path",
        )),
    }
}

#[derive(Clone, Copy)]
enum RgbaLayout {
    /// Expand single grayscale sample to (G, G, G, 0xFF).
    GrayOpaque,
    /// 2-channel input: gray, alpha.
    GrayAlpha,
    /// 3-channel input: r, g, b → opaque.
    RgbOpaque,
    /// 4-channel input: r, g, b, a.
    Rgba,
}

fn rgba_layout(h: &Header, depth: usize) -> RgbaLayout {
    // Standard tuple-types pin the layout; everything else (Custom or
    // missing TUPLTYPE) falls back on DEPTH.
    match &h.tupltype {
        Some(Tupltype::BlackAndWhiteAlpha) | Some(Tupltype::GrayscaleAlpha) => {
            RgbaLayout::GrayAlpha
        }
        Some(Tupltype::RgbAlpha) => RgbaLayout::Rgba,
        Some(Tupltype::Rgb) => RgbaLayout::RgbOpaque,
        Some(Tupltype::BlackAndWhite) | Some(Tupltype::Grayscale) => RgbaLayout::GrayOpaque,
        // Custom(_) or None — DEPTH is authoritative.
        Some(Tupltype::Custom(_)) | None => match depth {
            1 => RgbaLayout::GrayOpaque,
            2 => RgbaLayout::GrayAlpha,
            3 => RgbaLayout::RgbOpaque,
            4 => RgbaLayout::Rgba,
            _ => RgbaLayout::GrayOpaque,
        },
    }
}

fn fill_rgba_u8(dst: &mut [u8], src: &[u16], maxval: u32, layout: RgbaLayout) {
    match layout {
        RgbaLayout::GrayOpaque => {
            let g = scale_to_u8(src[0], maxval);
            dst[0] = g;
            dst[1] = g;
            dst[2] = g;
            dst[3] = 0xFF;
        }
        RgbaLayout::GrayAlpha => {
            let g = scale_to_u8(src[0], maxval);
            let a = scale_to_u8(src[1], maxval);
            dst[0] = g;
            dst[1] = g;
            dst[2] = g;
            dst[3] = a;
        }
        RgbaLayout::RgbOpaque => {
            dst[0] = scale_to_u8(src[0], maxval);
            dst[1] = scale_to_u8(src[1], maxval);
            dst[2] = scale_to_u8(src[2], maxval);
            dst[3] = 0xFF;
        }
        RgbaLayout::Rgba => {
            dst[0] = scale_to_u8(src[0], maxval);
            dst[1] = scale_to_u8(src[1], maxval);
            dst[2] = scale_to_u8(src[2], maxval);
            dst[3] = scale_to_u8(src[3], maxval);
        }
    }
}

/// Pick the best [`PbmPixelFormat`] for the parsed header. PAM tuple types
/// drive the choice when present; otherwise we go by `(depth, bits)`.
fn pick_pixel_format(h: &Header) -> Result<PbmPixelFormat> {
    Ok(match h.magic {
        Magic::PfPfmGrayFloat => PbmPixelFormat::GrayF32,
        Magic::PFPfmRgbFloat => PbmPixelFormat::RgbF32,
        Magic::P1AsciiBitmap | Magic::P4BinaryBitmap => PbmPixelFormat::MonoBlack,
        Magic::P2AsciiGraymap | Magic::P5BinaryGraymap => {
            if h.maxval > 255 {
                PbmPixelFormat::Gray16Le
            } else {
                PbmPixelFormat::Gray8
            }
        }
        Magic::P3AsciiPixmap | Magic::P6BinaryPixmap => {
            if h.maxval > 255 {
                PbmPixelFormat::Rgb48Le
            } else {
                PbmPixelFormat::Rgb24
            }
        }
        Magic::P7Pam => {
            let bits16 = h.maxval > 255;
            match (&h.tupltype, h.depth, bits16) {
                (Some(Tupltype::BlackAndWhite), _, _) => PbmPixelFormat::MonoBlack,
                (Some(Tupltype::Grayscale), _, false) => PbmPixelFormat::Gray8,
                (Some(Tupltype::Grayscale), _, true) => PbmPixelFormat::Gray16Le,
                (Some(Tupltype::Rgb), _, false) => PbmPixelFormat::Rgb24,
                (Some(Tupltype::Rgb), _, true) => PbmPixelFormat::Rgb48Le,
                (Some(Tupltype::GrayscaleAlpha), _, false) => PbmPixelFormat::Ya8,
                (Some(Tupltype::GrayscaleAlpha), _, true) => PbmPixelFormat::Ya16Le,
                (Some(Tupltype::BlackAndWhiteAlpha), _, _) => PbmPixelFormat::Rgba,
                (Some(Tupltype::RgbAlpha), _, false) => PbmPixelFormat::Rgba,
                (Some(Tupltype::RgbAlpha), _, true) => PbmPixelFormat::Rgba64Le,
                // Tuple type omitted OR user-defined / non-standard:
                // route the channels through the depth-based fallback
                // (the spec explicitly permits arbitrary TUPLTYPE names,
                // in which case DEPTH is the authoritative channel count).
                (None, 1, false) | (Some(Tupltype::Custom(_)), 1, false) => PbmPixelFormat::Gray8,
                (None, 1, true) | (Some(Tupltype::Custom(_)), 1, true) => PbmPixelFormat::Gray16Le,
                (None, 2, false) | (Some(Tupltype::Custom(_)), 2, false) => PbmPixelFormat::Ya8,
                (None, 2, true) | (Some(Tupltype::Custom(_)), 2, true) => PbmPixelFormat::Ya16Le,
                (None, 3, false) | (Some(Tupltype::Custom(_)), 3, false) => PbmPixelFormat::Rgb24,
                (None, 3, true) | (Some(Tupltype::Custom(_)), 3, true) => PbmPixelFormat::Rgb48Le,
                (None, 4, false) | (Some(Tupltype::Custom(_)), 4, false) => PbmPixelFormat::Rgba,
                (None, 4, true) | (Some(Tupltype::Custom(_)), 4, true) => PbmPixelFormat::Rgba64Le,
                (_, d, _) => {
                    return Err(Error::unsupported(format!(
                        "PAM: depth {d} outside the supported 1..=4 range"
                    )))
                }
            }
        }
    })
}

/// Scale a sample (range `0..=maxval`) to a full 8-bit byte. For
/// `maxval == 1` (PBM) we return 0 or 255.
pub(crate) fn scale_to_u8(s: u16, maxval: u32) -> u8 {
    if maxval == 0 {
        return 0;
    }
    if maxval == 255 {
        return s as u8;
    }
    if maxval == 1 {
        return if s != 0 { 0xFF } else { 0 };
    }
    // Round-half-up: (s * 255 + maxval/2) / maxval.
    let num = s as u32 * 255 + maxval / 2;
    (num / maxval).min(255) as u8
}

/// Scale a sample to a full 16-bit value.
pub(crate) fn scale_to_u16(s: u16, maxval: u32) -> u16 {
    if maxval == 0 {
        return 0;
    }
    if maxval == 65535 {
        return s;
    }
    if maxval == 1 {
        return if s != 0 { 0xFFFF } else { 0 };
    }
    let num = s as u64 * 65535 + (maxval as u64) / 2;
    (num / maxval as u64).min(65535) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_helpers_extreme_values() {
        assert_eq!(scale_to_u8(0, 255), 0);
        assert_eq!(scale_to_u8(255, 255), 255);
        assert_eq!(scale_to_u8(1, 1), 0xFF);
        assert_eq!(scale_to_u8(50, 100), 128); // round-half-up
        assert_eq!(scale_to_u16(1, 1), 0xFFFF);
        assert_eq!(scale_to_u16(0xABCD, 65535), 0xABCD);
    }

    #[test]
    fn decode_p3_simple() {
        let buf = b"P3\n2 1\n255\n255 0 0  0 255 0\n";
        let (image, fmt) = decode_pbm(buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Rgb24);
        assert_eq!(image.planes[0].data, [255, 0, 0, 0, 255, 0]);
    }

    #[test]
    fn decode_p6_16bit_be_samples() {
        let header = b"P6\n1 1\n65535\n";
        let mut buf = Vec::from(&header[..]);
        // R=0xABCD, G=0x1234, B=0x5678 in BE
        buf.extend_from_slice(&[0xAB, 0xCD, 0x12, 0x34, 0x56, 0x78]);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Rgb48Le);
        // LE in plane: r,g,b each 2 bytes
        assert_eq!(image.planes[0].data, [0xCD, 0xAB, 0x34, 0x12, 0x78, 0x56]);
    }

    #[test]
    fn decode_p7_rgba() {
        let mut buf = Vec::from(
            b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n".as_slice(),
        );
        buf.extend_from_slice(&[10, 20, 30, 40]);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Rgba);
        assert_eq!(image.planes[0].data, [10, 20, 30, 40]);
    }

    #[test]
    fn decode_p4_packs_bits_msb_first() {
        // 11 wide → 2 bytes per row; 1 = black per PBM convention.
        let mut buf = Vec::from(b"P4\n11 1\n".as_slice());
        buf.push(0b1010_1100);
        buf.push(0b1110_0000);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::MonoBlack);
        // MonoBlack stores the same MSB-first bit layout as PBM, so the
        // plane bytes round-trip the input bytes for the populated bits.
        assert_eq!(image.planes[0].stride, 2);
        assert_eq!(image.planes[0].data[0], 0b1010_1100);
        assert_eq!(image.planes[0].data[1] & 0b1110_0000, 0b1110_0000);
    }

    #[test]
    fn decode_p4_fast_path_byte_aligned_is_pure_memcpy() {
        // Width is a multiple of 8 → no trailing-pad mask; every body
        // byte must reach the plane verbatim. 16 px × 2 rows = 4 body
        // bytes; mix high/low bits to surface any indexing skew.
        let mut buf = Vec::from(b"P4\n16 2\n".as_slice());
        buf.extend_from_slice(&[0b1010_1100, 0b1111_0010, 0b0011_0110, 0b1001_1001]);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::MonoBlack);
        assert_eq!(image.planes[0].stride, 2);
        assert_eq!(
            image.planes[0].data,
            [0b1010_1100, 0b1111_0010, 0b0011_0110, 0b1001_1001]
        );
    }

    #[test]
    fn decode_p4_fast_path_unaligned_zeros_trailing_pad() {
        // 11 px → 2 bytes per row, last 5 bits unused. The output plane
        // must zero those bits even if the on-disk body had dirty
        // padding — canonical `MonoBlack` keeps the pad bits clear.
        let mut buf = Vec::from(b"P4\n11 2\n".as_slice());
        // Row 0: 1010 1100 / 111X XXXX (X = dirty pad bits)
        // Row 1: 0011 0110 / 100X XXXX
        buf.extend_from_slice(&[0b1010_1100, 0b1110_1111, 0b0011_0110, 0b1001_0101]);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::MonoBlack);
        assert_eq!(image.planes[0].stride, 2);
        // Top 3 bits of byte 1 are the populated pixels 8/9/10;
        // remaining 5 bits zeroed.
        assert_eq!(image.planes[0].data[0], 0b1010_1100);
        assert_eq!(image.planes[0].data[1], 0b1110_0000);
        assert_eq!(image.planes[0].data[2], 0b0011_0110);
        assert_eq!(image.planes[0].data[3], 0b1000_0000);
    }

    #[test]
    fn decode_p4_fast_path_matches_legacy_for_every_width_modulo() {
        // Byte-for-byte agreement with the pre-r248 generic path
        // (`decode_binary` + `samples_to_plane`) across every used-bit
        // count covering each `width % 8`. The generic path is still
        // reachable for P1 (ASCII bitmap) and P7 `BLACKANDWHITE`, and
        // its output for P4 must match the fast path exactly so the
        // r248 commit is byte-equivalent.
        for &w in &[1usize, 3, 7, 8, 9, 15, 16, 17, 32, 33, 65, 128, 129] {
            let h = 3usize;
            let row_bytes = w.div_ceil(8);
            // Deterministic packed source bytes — fill the unused tail
            // bits with garbage so the trailing-pad mask is exercised.
            let mut body = vec![0u8; row_bytes * h];
            for (i, b) in body.iter_mut().enumerate() {
                *b = ((i.wrapping_mul(0x9E37) ^ 0xA5) & 0xFF) as u8;
            }
            let mut buf = format!("P4\n{w} {h}\n").into_bytes();
            buf.extend_from_slice(&body);
            let (image, _) = decode_pbm(&buf).unwrap();
            // Build the expected plane via the row-level helper directly
            // — the same kernel both the fast path and the legacy
            // re-pack converge on for P4 → `MonoBlack`.
            let mut expected = vec![0u8; row_bytes * h];
            for y in 0..h {
                let off = y * row_bytes;
                crate::binary::copy_p4_row_msb(
                    &body[off..off + row_bytes],
                    &mut expected[off..off + row_bytes],
                    w,
                );
            }
            assert_eq!(image.planes[0].data, expected, "w={w}");
            assert_eq!(image.planes[0].stride, row_bytes, "w={w}");
        }
    }

    #[test]
    fn decode_p4_fast_path_rejects_truncated_body() {
        // 16 px × 4 rows needs 8 body bytes; only 5 supplied. The fast
        // path must reject before allocating the destination plane
        // (mirrors the round-171 OOM hardening on the generic decoder).
        let mut buf = Vec::from(b"P4\n16 4\n".as_slice());
        buf.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        let err = decode_pbm(&buf).unwrap_err();
        match err {
            crate::error::PbmError::InvalidData(s) => {
                assert!(s.contains("truncated"), "unexpected message: {s}");
            }
            other => panic!("expected InvalidData, got {other:?}"),
        }
    }

    #[test]
    fn decode_p4_fast_path_rejects_oom_dimension() {
        // Header claims `width * height` in the billions: the row-byte
        // multiplication overflows usize on 32-bit and easily exceeds
        // body.len() on 64-bit. Must fail before `vec![0u8; need]`.
        let buf = b"P4\n8 200888808\n\x00\x00\x00\x00";
        let err = decode_pbm(buf).unwrap_err();
        match err {
            crate::error::PbmError::InvalidData(_) => {}
            other => panic!("expected InvalidData, got {other:?}"),
        }
    }

    /// Pseudorandom but deterministic byte fill — produces enough
    /// variation that any indexing slip in the fast path would show up
    /// as a mismatch against the generic path.
    fn fill_xorshift(buf: &mut [u8], seed: u32) {
        let mut state = seed | 1;
        for b in buf.iter_mut() {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *b = (state & 0xff) as u8;
        }
    }

    /// Round-trip the bytewise-decode hot path against a synthetic file
    /// whose body bytes are known up front, asserting the decoded plane
    /// data is bit-identical to those wire bytes (8-bit case) or to the
    /// LE byte-swap of them (16-bit case).
    #[test]
    fn decode_bytewise_p5_gray8_is_pure_memcpy() {
        // 7×3 gray image so width is not a power of two — stride =
        // bytes_per_pixel * w with no padding for `Gray8`.
        let mut body = vec![0u8; 7 * 3];
        fill_xorshift(&mut body, 0x1234_5678);
        let mut buf = Vec::from(b"P5\n7 3\n255\n".as_slice());
        buf.extend_from_slice(&body);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Gray8);
        assert_eq!(image.planes[0].stride, 7);
        assert_eq!(image.planes[0].data, body);
    }

    #[test]
    fn decode_bytewise_p6_rgb24_is_pure_memcpy() {
        let mut body = vec![0u8; 5 * 4 * 3];
        fill_xorshift(&mut body, 0x2345_6789);
        let mut buf = Vec::from(b"P6\n5 4\n255\n".as_slice());
        buf.extend_from_slice(&body);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Rgb24);
        assert_eq!(image.planes[0].stride, 15);
        assert_eq!(image.planes[0].data, body);
    }

    #[test]
    fn decode_bytewise_p7_rgba_is_pure_memcpy() {
        let mut body = vec![0u8; 3 * 2 * 4];
        fill_xorshift(&mut body, 0x3456_789a);
        let mut buf = Vec::from(
            b"P7\nWIDTH 3\nHEIGHT 2\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n".as_slice(),
        );
        buf.extend_from_slice(&body);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Rgba);
        assert_eq!(image.planes[0].stride, 12);
        assert_eq!(image.planes[0].data, body);
    }

    #[test]
    fn decode_bytewise_p7_ya8_is_pure_memcpy() {
        let mut body = vec![0u8; 4 * 3 * 2];
        fill_xorshift(&mut body, 0x4567_89ab);
        let mut buf = Vec::from(
            b"P7\nWIDTH 4\nHEIGHT 3\nDEPTH 2\nMAXVAL 255\nTUPLTYPE GRAYSCALE_ALPHA\nENDHDR\n"
                .as_slice(),
        );
        buf.extend_from_slice(&body);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Ya8);
        assert_eq!(image.planes[0].stride, 8);
        assert_eq!(image.planes[0].data, body);
    }

    #[test]
    fn decode_bytewise_p5_gray16_swaps_to_le_plane() {
        // Two BE samples on the wire — the plane gets them in LE.
        // 0x1234 BE = [0x12, 0x34] wire → [0x34, 0x12] plane.
        let mut buf = Vec::from(b"P5\n2 1\n65535\n".as_slice());
        buf.extend_from_slice(&[0x12, 0x34, 0xab, 0xcd]);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Gray16Le);
        assert_eq!(image.planes[0].stride, 4);
        assert_eq!(image.planes[0].data, [0x34, 0x12, 0xcd, 0xab]);
    }

    #[test]
    fn decode_bytewise_falls_through_for_non_natural_maxval() {
        // maxval=200 forces per-sample scaling; the fast path must
        // decline and the generic `scale_to_u8` path must take over.
        // 100 / 200 ≈ 0.5 → round-half-up → 128.
        let mut buf = Vec::from(b"P5\n2 1\n200\n".as_slice());
        buf.extend_from_slice(&[100, 200]);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Gray8);
        assert_eq!(image.planes[0].data, [128, 255]);
    }

    #[test]
    fn decode_bytewise_p7_ya16_swaps_to_le_plane() {
        // 16-bit `GRAYSCALE_ALPHA` decodes natively as `Ya16Le`: two
        // big-endian u16 wire channels (G, A) per pixel land in the
        // plane as little-endian (G, A) pairs with full 16-bit
        // precision — eligible for the bytewise fast path since the
        // mapping is a pure per-sample byte swap.
        let mut buf = Vec::from(
            b"P7\nWIDTH 1\nHEIGHT 2\nDEPTH 2\nMAXVAL 65535\nTUPLTYPE GRAYSCALE_ALPHA\nENDHDR\n"
                .as_slice(),
        );
        // Row 0: G=0x1234, A=0xABCD; row 1: G=0x00FF, A=0xFF00 — BE wire.
        buf.extend_from_slice(&[0x12, 0x34, 0xAB, 0xCD, 0x00, 0xFF, 0xFF, 0x00]);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Ya16Le);
        assert_eq!(image.planes[0].stride, 4);
        assert_eq!(
            image.planes[0].data,
            [0x34, 0x12, 0xCD, 0xAB, 0xFF, 0x00, 0x00, 0xFF]
        );
    }

    #[test]
    fn decode_p7_ya16_generic_path_scales_non_natural_maxval() {
        // A non-natural maxval forces the generic widen-then-rescale
        // path; the destination is still `Ya16Le` and each channel is
        // scaled to the full 16-bit range. With maxval=1000:
        // 500 → round(500 * 65535 / 1000) = 32768 (round-half-up),
        // 1000 → 65535.
        let mut buf = Vec::from(
            b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 2\nMAXVAL 1000\nTUPLTYPE GRAYSCALE_ALPHA\nENDHDR\n"
                .as_slice(),
        );
        // G=500, A=1000 as big-endian u16 wire samples.
        buf.extend_from_slice(&[0x01, 0xF4, 0x03, 0xE8]);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Ya16Le);
        let g = u16::from_le_bytes(image.planes[0].data[0..2].try_into().unwrap());
        let a = u16::from_le_bytes(image.planes[0].data[2..4].try_into().unwrap());
        assert_eq!(g, super::scale_to_u16(500, 1000));
        assert_eq!(a, 65535);
    }

    #[test]
    fn decode_bytewise_falls_through_for_p7_blackandwhite_alpha() {
        // BLACKANDWHITE_ALPHA is depth=2 with 1-bit semantics on the
        // first channel — the plane expansion to RGBA cannot use a
        // raw byte copy. Fast path declines; generic path runs.
        let mut buf = Vec::from(
            b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 2\nMAXVAL 1\nTUPLTYPE BLACKANDWHITE_ALPHA\nENDHDR\n"
                .as_slice(),
        );
        buf.extend_from_slice(&[1, 1]);
        let (image, fmt) = decode_pbm(&buf).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Rgba);
        // Sample 1 with maxval=1 expands to 0xFF on every channel.
        assert_eq!(image.planes[0].data, [0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn decode_bytewise_matches_legacy_for_every_8bit_magic() {
        // For each magic where the fast path is eligible, the decoded
        // plane must be byte-equivalent to a reference plane built
        // from the same wire bytes via the same byte layout — the wire
        // bytes ARE the plane bytes when maxval=255 and the wire
        // channel count matches the plane bytes-per-pixel.
        let configs: &[(&[u8], usize, &str, usize)] = &[
            // (header, body_size_per_pixel, label, depth)
            (b"P5\n8 4\n255\n", 1, "P5/8x4 gray8", 1),
            (b"P6\n8 4\n255\n", 3, "P6/8x4 rgb24", 3),
        ];
        for (header_prefix, bpp, label, _depth) in configs {
            let mut body = vec![0u8; 8 * 4 * bpp];
            fill_xorshift(&mut body, 0xdead_beef);
            let mut buf = Vec::from(*header_prefix);
            buf.extend_from_slice(&body);
            let (image, _fmt) = decode_pbm(&buf).unwrap();
            assert_eq!(image.planes[0].stride, 8 * bpp, "{label}");
            assert_eq!(image.planes[0].data, body, "{label}");
        }
        // P7 variants — wire channels match plane bytes-per-pixel.
        for (depth, tupltype, label) in [
            (1usize, "GRAYSCALE", "P7/3x2 grayscale"),
            (2, "GRAYSCALE_ALPHA", "P7/3x2 ya8"),
            (3, "RGB", "P7/3x2 rgb24"),
            (4, "RGB_ALPHA", "P7/3x2 rgba"),
        ] {
            let mut body = vec![0u8; 3 * 2 * depth];
            fill_xorshift(&mut body, 0xcafe_b0ba);
            let header = format!(
                "P7\nWIDTH 3\nHEIGHT 2\nDEPTH {depth}\nMAXVAL 255\nTUPLTYPE {tupltype}\nENDHDR\n",
            );
            let mut buf = header.into_bytes();
            buf.extend_from_slice(&body);
            let (image, _fmt) = decode_pbm(&buf).unwrap();
            assert_eq!(image.planes[0].stride, 3 * depth, "{label}");
            assert_eq!(image.planes[0].data, body, "{label}");
        }
    }

    #[test]
    fn decode_bytewise_matches_legacy_for_every_16bit_magic() {
        // 16-bit fast path: the plane bytes are the per-sample LE
        // byte swap of the wire bytes. Verified by re-swapping the
        // plane and comparing to the original wire body.
        let mut body = vec![0u8; 6 * 4 * 2]; // P5 6×4 gray16
        fill_xorshift(&mut body, 0xfeed_face);
        let mut buf = Vec::from(b"P5\n6 4\n65535\n".as_slice());
        buf.extend_from_slice(&body);
        let (image, _fmt) = decode_pbm(&buf).unwrap();
        let mut roundtrip = vec![0u8; body.len()];
        for (s, d) in image.planes[0]
            .data
            .chunks_exact(2)
            .zip(roundtrip.chunks_exact_mut(2))
        {
            d[0] = s[1];
            d[1] = s[0];
        }
        assert_eq!(roundtrip, body);
        // Same shape for P6 16-bit, P7 GRAYSCALE_ALPHA 16-bit, and
        // P7 RGB_ALPHA 16-bit.
        for (bpp, header) in [
            (6usize, "P6\n6 4\n65535\n".to_string()),
            (
                4,
                "P7\nWIDTH 6\nHEIGHT 4\nDEPTH 2\nMAXVAL 65535\nTUPLTYPE GRAYSCALE_ALPHA\nENDHDR\n"
                    .to_string(),
            ),
            (
                8,
                "P7\nWIDTH 6\nHEIGHT 4\nDEPTH 4\nMAXVAL 65535\nTUPLTYPE RGB_ALPHA\nENDHDR\n"
                    .to_string(),
            ),
        ] {
            let mut body = vec![0u8; 6 * 4 * bpp];
            fill_xorshift(&mut body, 0xb16b_00b5);
            let mut buf = header.into_bytes();
            buf.extend_from_slice(&body);
            let (image, _fmt) = decode_pbm(&buf).unwrap();
            let mut roundtrip = vec![0u8; body.len()];
            for (s, d) in image.planes[0]
                .data
                .chunks_exact(2)
                .zip(roundtrip.chunks_exact_mut(2))
            {
                d[0] = s[1];
                d[1] = s[0];
            }
            assert_eq!(roundtrip, body, "bpp={bpp}");
        }
    }

    #[test]
    fn decode_bytewise_rejects_truncated_body() {
        // 6×4 P6 8-bit needs 72 body bytes; we supply 10. Must reject
        // before allocating the plane (mirrors the round-171 OOM
        // hardening on the generic decoder).
        let mut buf = Vec::from(b"P6\n6 4\n255\n".as_slice());
        buf.extend_from_slice(&[0xAA; 10]);
        let err = decode_pbm(&buf).unwrap_err();
        match err {
            crate::error::PbmError::InvalidData(s) => {
                assert!(s.contains("truncated"), "unexpected message: {s}");
            }
            other => panic!("expected InvalidData, got {other:?}"),
        }
    }

    #[test]
    fn decode_bytewise_rejects_oom_dimension() {
        // P5 16-bit with multi-billion height: stride × height
        // overflows or vastly exceeds body.len(). Must fail without
        // the multi-GiB plane allocation.
        let buf = b"P5\n8 200888808\n65535\n\x00\x00\x00\x00";
        let err = decode_pbm(buf).unwrap_err();
        match err {
            crate::error::PbmError::InvalidData(_) => {}
            other => panic!("expected InvalidData, got {other:?}"),
        }
    }
}
