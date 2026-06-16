//! Portable FloatMap (`Pf` / `PF`) decode + encode.
//!
//! The Portable FloatMap is the floating-point member of the Netpbm
//! family: a three-line ASCII header (magic, `width height`, scale)
//! followed by raw IEEE-754 binary32 samples — one per grayscale pixel
//! (`Pf`) or three interleaved R, G, B per colour pixel (`PF`). See
//! `docs/image/netpbm/pfm-portable-floatmap.md` (Debevec PFM reference).
//!
//! Two layout rules distinguish it from the integer PNM body formats:
//!
//! * **Byte order** is selected by the sign of the header's scale line —
//!   negative ⇒ little-endian, positive ⇒ big-endian — and applies to
//!   every 4-byte float sample.
//! * **Row order is bottom-to-top**: the first row of samples in the
//!   file is the *bottom* row of the image. This module flips rows so the
//!   in-memory [`PbmImage`] plane is the conventional top-to-bottom
//!   layout.
//!
//! In memory the samples are always stored **little-endian** (the
//! [`PbmPixelFormat::GrayF32`] / [`PbmPixelFormat::RgbF32`] contract),
//! independent of the on-disk byte order, so decode/encode only ever
//! byte-swaps when the on-disk order is big-endian.

use crate::error::{PbmError as Error, Result};
use crate::header::{parse_header, Header, Magic, PfmInfo};
use crate::image::{PbmImage, PbmPixelFormat, PbmPlane};

/// Byte order + scale recovered from a decoded Portable FloatMap header.
///
/// The decoder preserves the raw float sample values unchanged; `scale`
/// is the producer's advisory factor (the absolute value of the header's
/// third line), reported as metadata rather than applied to the pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PfmHeaderInfo {
    /// `true` when the on-disk samples were little-endian.
    pub little_endian: bool,
    /// Absolute value of the scale line (the application-defined scale
    /// factor).
    pub scale: f32,
    /// Channels per pixel: 1 for `Pf`, 3 for `PF`.
    pub channels: u32,
}

/// Decode a complete Portable FloatMap file into a [`PbmImage`] plus the
/// recovered [`PfmHeaderInfo`]. Errors if the input is not a `Pf` / `PF`
/// stream.
pub fn decode_pfm(input: &[u8]) -> Result<(PbmImage, PfmHeaderInfo)> {
    decode_pfm_consumed(input).map(|(image, info, _consumed)| (image, info))
}

/// Decode the first Portable FloatMap image in `input` and additionally
/// report how many bytes it occupied on disk — the header length plus the
/// exact `width * height * channels * 4` IEEE-754 binary32 raster (per the
/// "Raster (sample) layout" section of the Debevec PFM reference, which
/// fixes the total raster size at `xres * yres * channels * 4` bytes).
///
/// This is the Portable FloatMap analogue of
/// [`crate::decode_pbm_consumed`]: the returned `usize` lets a caller walk
/// a stream of concatenated PFM images while keeping the rich
/// [`PfmHeaderInfo`] (byte order + scale + channel count) for each image —
/// information the integer-flavoured `decode_pbm_consumed` does not carry.
/// Advance a cursor by the returned count to reach the next image's magic:
///
/// ```
/// # use oxideav_pbm::{decode_pfm_consumed, encode_pfm, PbmImage, PbmPixelFormat, PbmPlane};
/// # let img = PbmImage {
/// #     width: 1, height: 1, pixel_format: PbmPixelFormat::GrayF32,
/// #     planes: vec![PbmPlane { stride: 4, data: 1.0f32.to_le_bytes().to_vec() }],
/// #     pts: None,
/// # };
/// let mut stream = encode_pfm(&img, true, 1.0).unwrap();
/// stream.extend(encode_pfm(&img, false, 2.0).unwrap());
/// let mut off = 0;
/// while off < stream.len() {
///     let (_image, info, consumed) = decode_pfm_consumed(&stream[off..]).unwrap();
///     println!("scale={} le={}", info.scale, info.little_endian);
///     off += consumed;
/// }
/// ```
pub fn decode_pfm_consumed(input: &[u8]) -> Result<(PbmImage, PfmHeaderInfo, usize)> {
    let header = parse_header(input)?;
    let info = header
        .pfm
        .ok_or_else(|| Error::invalid("PFM: input is not a Portable FloatMap (Pf/PF) stream"))?;
    let body = &input[header.data_offset..];
    let (image, _fmt) = decode_pfm_image(&header, body)?;
    // On-disk raster size is exactly width * height * channels * 4 bytes;
    // decode_pfm_image already validated that `body` holds at least this
    // many bytes, so the multiply cannot overflow past what was indexed.
    let body_len = (header.width as usize)
        .checked_mul(header.height as usize)
        .and_then(|v| v.checked_mul(header.depth as usize))
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| Error::invalid("PFM: raster-size overflow"))?;
    Ok((
        image,
        PfmHeaderInfo {
            little_endian: info.little_endian,
            scale: info.scale,
            channels: header.depth,
        },
        header.data_offset + body_len,
    ))
}

/// Decode the raw float body of an already-parsed PFM [`Header`]. `body`
/// starts at `header.data_offset`. Shared by [`decode_pfm`] and the
/// unified [`crate::decode_pbm`] entry point.
pub(crate) fn decode_pfm_image(h: &Header, body: &[u8]) -> Result<(PbmImage, PbmPixelFormat)> {
    let info: PfmInfo = h
        .pfm
        .ok_or_else(|| Error::invalid("PFM: missing byte-order metadata"))?;
    let w = h.width as usize;
    let hh = h.height as usize;
    let ch = h.depth as usize;
    if w == 0 || hh == 0 || ch == 0 {
        return Err(Error::invalid("PFM: zero dimension"));
    }

    // row_bytes = width * channels * 4, with overflow guards before any
    // allocation so a malformed header cannot trigger a huge `vec!`.
    let row_bytes = w
        .checked_mul(ch)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| Error::invalid("PFM: row-size overflow"))?;
    let need = row_bytes
        .checked_mul(hh)
        .ok_or_else(|| Error::invalid("PFM: raster-size overflow"))?;
    if body.len() < need {
        return Err(Error::invalid("PFM: float data truncated"));
    }

    let format = if ch == 3 {
        PbmPixelFormat::RgbF32
    } else {
        PbmPixelFormat::GrayF32
    };
    let stride = row_bytes;
    let mut data = vec![0u8; need];

    // File rows run bottom-to-top: file row 0 is the image's bottom row,
    // which lands at in-memory row hh-1. Normalise big-endian samples to
    // the little-endian in-memory contract on the way in. The BE swap
    // funnels through `swap_bytes_u32_row` so the inner loop walks
    // `chunks_exact(4)` over `[u8; 4]` blocks LLVM can autovectorize.
    for file_row in 0..hh {
        let mem_row = hh - 1 - file_row;
        let src = &body[file_row * row_bytes..file_row * row_bytes + row_bytes];
        let dst = &mut data[mem_row * stride..mem_row * stride + row_bytes];
        if info.little_endian {
            dst.copy_from_slice(src);
        } else {
            swap_bytes_u32_row(src, dst);
        }
    }

    Ok((
        PbmImage {
            width: h.width,
            height: h.height,
            pixel_format: format,
            planes: vec![PbmPlane { stride, data }],
            pts: None,
        },
        format,
    ))
}

/// Decode a Portable FloatMap and **apply** the header's scale factor to
/// every sample, returning the scaled [`PbmImage`] plus the recovered
/// [`PfmHeaderInfo`].
///
/// Per the Debevec PFM reference the absolute value of the third header
/// line "serves as a scale factor … that an application may use to scale
/// sample values." [`decode_pfm`] deliberately leaves that application to
/// the caller (it preserves the factor as metadata and returns the raw
/// samples unchanged); this convenience wrapper performs the documented
/// multiply for callers that want the scaled linear-light values directly.
///
/// The returned [`PfmHeaderInfo::scale`] is still the header's original
/// factor — it is reported, not reset to `1.0`, so a caller can tell the
/// scaling has been applied and avoid double-applying it. Because the
/// factor has already been multiplied in, re-encoding the returned image
/// with [`encode_pfm`] and a scale of `1.0` reproduces the same linear
/// values.
pub fn decode_pfm_scaled(input: &[u8]) -> Result<(PbmImage, PfmHeaderInfo)> {
    let (mut image, info) = decode_pfm(input)?;
    apply_pfm_scale(&mut image, info.scale)?;
    Ok((image, info))
}

/// Multiply every IEEE-754 float sample of a [`PbmPixelFormat::GrayF32`]
/// or [`PbmPixelFormat::RgbF32`] image by `scale`, in place.
///
/// This is the documented PFM scale-factor application: the Debevec
/// reference says the magnitude of the header's third line is "a scale
/// factor … that an application may use to scale sample values." The
/// crate's decoders never apply it automatically (the factor is advisory),
/// so this helper lets a caller opt in. The samples are read and written
/// in the plane's little-endian in-memory contract; the alpha-free float
/// formats carry no channel that should be left unscaled, so every sample
/// is multiplied.
///
/// `scale` must be finite (a `NaN` or `±inf` factor would poison the whole
/// raster); a non-float pixel format is rejected. A `scale` of exactly
/// `1.0` is a no-op fast path.
pub fn apply_pfm_scale(image: &mut PbmImage, scale: f32) -> Result<()> {
    match image.pixel_format {
        PbmPixelFormat::GrayF32 | PbmPixelFormat::RgbF32 => {}
        other => {
            return Err(Error::unsupported(format!(
                "PFM scale: pixel format {other:?} is not a float map"
            )))
        }
    }
    if !scale.is_finite() {
        return Err(Error::invalid("PFM scale: factor must be finite"));
    }
    if scale == 1.0 {
        return Ok(());
    }
    for plane in &mut image.planes {
        for sample in plane.data.chunks_exact_mut(4) {
            let v = f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]);
            sample.copy_from_slice(&(v * scale).to_le_bytes());
        }
    }
    Ok(())
}

/// Divide every IEEE-754 float sample of a [`PbmPixelFormat::GrayF32`]
/// or [`PbmPixelFormat::RgbF32`] image by `scale`, in place — the inverse
/// of [`apply_pfm_scale`].
///
/// This is the encode-side companion to the documented PFM scale factor.
/// The Debevec reference says the magnitude of the header's third line is
/// "a scale factor … that an application may use to scale sample values",
/// and [`apply_pfm_scale`] performs that read-side multiply
/// (stored × scale ⇒ linear). To *store* linear values `L` under a chosen
/// factor `s` so that a reader applying the factor recovers `L`, the
/// writer must persist `L / s`; this helper performs that division so the
/// pair `apply_inverse_pfm_scale(.., s)` then `apply_pfm_scale(.., s)`
/// round-trips a sample back to itself (within `f32` precision).
///
/// Like [`apply_pfm_scale`], `scale` must be finite and a non-float pixel
/// format is rejected. `scale` must additionally be non-zero (division by
/// zero would poison the raster), matching what [`encode_pfm`] will write.
/// A `scale` of exactly `1.0` is a no-op fast path.
pub fn apply_inverse_pfm_scale(image: &mut PbmImage, scale: f32) -> Result<()> {
    match image.pixel_format {
        PbmPixelFormat::GrayF32 | PbmPixelFormat::RgbF32 => {}
        other => {
            return Err(Error::unsupported(format!(
                "PFM inverse scale: pixel format {other:?} is not a float map"
            )))
        }
    }
    if !scale.is_finite() {
        return Err(Error::invalid("PFM inverse scale: factor must be finite"));
    }
    if scale == 0.0 {
        return Err(Error::invalid("PFM inverse scale: factor must be non-zero"));
    }
    if scale == 1.0 {
        return Ok(());
    }
    for plane in &mut image.planes {
        for sample in plane.data.chunks_exact_mut(4) {
            let v = f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]);
            sample.copy_from_slice(&(v / scale).to_le_bytes());
        }
    }
    Ok(())
}

/// Encode a [`PbmImage`] of linear-light float samples as a Portable
/// FloatMap **carrying** the chosen `scale` factor, dividing the samples
/// by it on the way to disk.
///
/// This is the scale-folding counterpart to [`decode_pfm_scaled`]. Where
/// [`encode_pfm`] writes the raw samples verbatim and records `scale` only
/// as advisory metadata, this entry treats the input samples as the
/// already-scaled linear values `L` and stores `L / scale`, so that a
/// reader which applies the factor — [`decode_pfm_scaled`], or
/// [`decode_pfm`] followed by [`apply_pfm_scale`] with the recovered
/// factor — reproduces the original `L` (within `f32` precision). A
/// `scale` of `1.0` is identical to [`encode_pfm`].
///
/// The input image is not mutated: the division happens on an internal
/// copy of each plane. `little_endian` selects the on-disk byte order and
/// the sign of the scale line exactly as in [`encode_pfm`]; `scale` must
/// be finite and non-zero.
pub fn encode_pfm_scaled(image: &PbmImage, little_endian: bool, scale: f32) -> Result<Vec<u8>> {
    match image.pixel_format {
        PbmPixelFormat::GrayF32 | PbmPixelFormat::RgbF32 => {}
        other => {
            return Err(Error::unsupported(format!(
                "PFM encoder: pixel format {other:?} is not a float map"
            )))
        }
    }
    if !scale.is_finite() || scale == 0.0 {
        return Err(Error::invalid(
            "PFM encoder: scale must be finite and non-zero",
        ));
    }
    // Fold the factor out of the linear samples on an owned copy so the
    // caller's image is left untouched, then emit with the same factor in
    // the header. `apply_inverse_pfm_scale` validates the (already-checked)
    // format/scale again, which is cheap; the `1.0` no-op fast path makes
    // the unit-scale case a plain clone + `encode_pfm`.
    let mut scaled = image.clone();
    apply_inverse_pfm_scale(&mut scaled, scale)?;
    encode_pfm(&scaled, little_endian, scale)
}

/// Encode a [`PbmImage`] (whose pixel format must be
/// [`PbmPixelFormat::GrayF32`] or [`PbmPixelFormat::RgbF32`]) as a
/// Portable FloatMap. `little_endian` selects the on-disk byte order
/// (and therefore the sign of the scale line); `scale` is the
/// application-defined scale factor magnitude written on the third
/// header line.
pub fn encode_pfm(image: &PbmImage, little_endian: bool, scale: f32) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("PFM encoder: empty plane"));
    }
    encode_pfm_plane(
        &image.planes[0],
        image.pixel_format,
        image.width,
        image.height,
        little_endian,
        scale,
    )
}

/// Plane-level Portable FloatMap encoder. Lower-level than [`encode_pfm`]
/// for callers that already hold the plane bytes.
pub fn encode_pfm_plane(
    plane: &PbmPlane,
    format: PbmPixelFormat,
    width: u32,
    height: u32,
    little_endian: bool,
    scale: f32,
) -> Result<Vec<u8>> {
    let ch = match format {
        PbmPixelFormat::GrayF32 => 1usize,
        PbmPixelFormat::RgbF32 => 3usize,
        other => {
            return Err(Error::unsupported(format!(
                "PFM encoder: pixel format {other:?} is not a float map"
            )))
        }
    };
    if !scale.is_finite() || scale == 0.0 {
        return Err(Error::invalid(
            "PFM encoder: scale must be finite and non-zero",
        ));
    }
    let w = width as usize;
    let h = height as usize;
    let row_bytes = w
        .checked_mul(ch)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| Error::invalid("PFM encoder: row-size overflow"))?;
    if plane.stride < row_bytes {
        return Err(Error::invalid(
            "PFM encoder: plane stride smaller than a row",
        ));
    }
    if plane.data.len() < plane.stride * h {
        return Err(Error::invalid("PFM encoder: plane truncated"));
    }

    // The scale-line sign encodes the byte order; its magnitude is the
    // scale factor.
    let signed_scale = if little_endian {
        -scale.abs()
    } else {
        scale.abs()
    };
    // Route the PFM magic literal through the typed `Magic::wire_bytes`
    // accessor — same shape as the PNM encoder's `header_pnm` after
    // round 266 so the on-disk identifier is selected by variant rather
    // than by an open-coded `b"PF" / b"Pf"` table. The two PFM variants
    // are case-sensitive (`Pf` = grayscale, `PF` = RGB) and the typed
    // accessor preserves that distinction.
    let magic = if ch == 3 {
        Magic::PFPfmRgbFloat
    } else {
        Magic::PfPfmGrayFloat
    };

    let mut out = Vec::with_capacity(3 + 24 + row_bytes * h);
    out.extend_from_slice(magic.wire_bytes());
    out.push(b'\n');
    out.extend_from_slice(format!("{w} {h}\n").as_bytes());
    out.extend_from_slice(format_scale(signed_scale).as_bytes());
    out.push(b'\n');

    // Emit rows bottom-to-top: the first row written is in-memory row
    // h-1 (the image's bottom row). Swap to big-endian on the way out
    // when requested. The BE row swap is funneled through a row-level
    // helper so the inner loop runs over `[u8; 4]` chunks the compiler
    // can autovectorize (aarch64 `REV32`, x86 `BSWAP` / `PSHUFB`).
    for file_row in 0..h {
        let mem_row = h - 1 - file_row;
        let src = &plane.data[mem_row * plane.stride..mem_row * plane.stride + row_bytes];
        if little_endian {
            out.extend_from_slice(src);
        } else {
            let dst_start = out.len();
            out.resize(dst_start + row_bytes, 0);
            swap_bytes_u32_row(src, &mut out[dst_start..]);
        }
    }
    Ok(out)
}

/// Row-level byte-swap for 4-byte float samples. `src` and `dst` must be
/// the same length and a multiple of 4. The inner loop walks
/// `chunks_exact(4)` over both sides so LLVM lowers it to vector
/// `swap_bytes` (`REV32.16B` on aarch64; `pshufb` / `vpshufb` on x86).
#[inline]
fn swap_bytes_u32_row(src: &[u8], dst: &mut [u8]) {
    debug_assert_eq!(src.len(), dst.len());
    debug_assert_eq!(src.len() % 4, 0);
    for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        let v = u32::from_le_bytes([s[0], s[1], s[2], s[3]]).swap_bytes();
        d.copy_from_slice(&v.to_le_bytes());
    }
}

/// Format a scale value so it always carries a decimal point (or
/// exponent), matching the `1.0` / `-1.0` convention shown in the PFM
/// reference.
fn format_scale(v: f32) -> String {
    let s = format!("{v}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a GrayF32 / RgbF32 image whose samples encode their (row,
    /// col, channel) coordinates so a vertical flip is observable.
    fn float_image(w: u32, h: u32, ch: usize) -> PbmImage {
        let format = if ch == 3 {
            PbmPixelFormat::RgbF32
        } else {
            PbmPixelFormat::GrayF32
        };
        let stride = w as usize * ch * 4;
        let mut data = vec![0u8; stride * h as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                for c in 0..ch {
                    let v = (y * 1000 + x * 10 + c) as f32 + 0.5;
                    let off = y * stride + (x * ch + c) * 4;
                    data[off..off + 4].copy_from_slice(&v.to_le_bytes());
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

    #[test]
    fn roundtrip_gray_little_endian() {
        let img = float_image(5, 4, 1);
        let bytes = encode_pfm(&img, true, 1.0).unwrap();
        assert!(bytes.starts_with(b"Pf\n5 4\n-1.0\n"));
        let (back, info) = decode_pfm(&bytes).unwrap();
        assert!(info.little_endian);
        assert_eq!(info.scale, 1.0);
        assert_eq!(info.channels, 1);
        assert_eq!(back.planes[0].data, img.planes[0].data);
    }

    #[test]
    fn roundtrip_gray_big_endian() {
        let img = float_image(3, 6, 1);
        let bytes = encode_pfm(&img, false, 1.0).unwrap();
        assert!(bytes.starts_with(b"Pf\n3 6\n1.0\n"));
        let (back, info) = decode_pfm(&bytes).unwrap();
        assert!(!info.little_endian);
        assert_eq!(back.planes[0].data, img.planes[0].data);
    }

    #[test]
    fn roundtrip_rgb_little_endian() {
        let img = float_image(4, 3, 3);
        let bytes = encode_pfm(&img, true, 1.0).unwrap();
        assert!(bytes.starts_with(b"PF\n4 3\n-1.0\n"));
        let (back, info) = decode_pfm(&bytes).unwrap();
        assert_eq!(info.channels, 3);
        assert_eq!(back.pixel_format, PbmPixelFormat::RgbF32);
        assert_eq!(back.planes[0].data, img.planes[0].data);
    }

    #[test]
    fn roundtrip_rgb_big_endian() {
        let img = float_image(2, 2, 3);
        let bytes = encode_pfm(&img, false, 1.0).unwrap();
        let (back, _info) = decode_pfm(&bytes).unwrap();
        assert_eq!(back.planes[0].data, img.planes[0].data);
    }

    #[test]
    fn roundtrip_non_unit_scale() {
        let img = float_image(2, 2, 1);
        let bytes = encode_pfm(&img, false, 2.5).unwrap();
        assert!(bytes.starts_with(b"Pf\n2 2\n2.5\n"));
        let (_back, info) = decode_pfm(&bytes).unwrap();
        assert_eq!(info.scale, 2.5);
        assert!(!info.little_endian);
    }

    #[test]
    fn bottom_to_top_flip_is_correct() {
        // A 1×2 grayscale image: memory row 0 holds 11.0, row 1 holds
        // 22.0. On disk the bottom row (memory row 1 = 22.0) must come
        // first.
        let mut data = vec![0u8; 2 * 4];
        data[0..4].copy_from_slice(&11.0f32.to_le_bytes());
        data[4..8].copy_from_slice(&22.0f32.to_le_bytes());
        let img = PbmImage {
            width: 1,
            height: 2,
            pixel_format: PbmPixelFormat::GrayF32,
            planes: vec![PbmPlane { stride: 4, data }],
            pts: None,
        };
        let bytes = encode_pfm(&img, true, 1.0).unwrap();
        let body = &bytes[bytes.len() - 8..];
        // First on-disk sample is the bottom row = 22.0.
        assert_eq!(f32::from_le_bytes(body[0..4].try_into().unwrap()), 22.0);
        assert_eq!(f32::from_le_bytes(body[4..8].try_into().unwrap()), 11.0);
        // And it flips back on decode.
        let (back, _) = decode_pfm(&bytes).unwrap();
        assert_eq!(
            f32::from_le_bytes(back.planes[0].data[0..4].try_into().unwrap()),
            11.0
        );
    }

    #[test]
    fn big_endian_disk_bytes_are_swapped() {
        let mut data = vec![0u8; 4];
        data.copy_from_slice(&1.0f32.to_le_bytes()); // LE: 00 00 80 3F
        let img = PbmImage {
            width: 1,
            height: 1,
            pixel_format: PbmPixelFormat::GrayF32,
            planes: vec![PbmPlane { stride: 4, data }],
            pts: None,
        };
        let bytes = encode_pfm(&img, false, 1.0).unwrap();
        // On disk, big-endian 1.0 = 3F 80 00 00.
        assert_eq!(&bytes[bytes.len() - 4..], &[0x3F, 0x80, 0x00, 0x00]);
    }

    #[test]
    fn decode_rejects_truncated_body() {
        let buf = b"PF\n4 4\n-1.0\n\x00\x00\x00\x00"; // only one sample
        assert!(decode_pfm(buf).is_err());
    }

    #[test]
    fn encode_rejects_non_float_format() {
        let plane = PbmPlane {
            stride: 1,
            data: vec![0u8],
        };
        let err = encode_pfm_plane(&plane, PbmPixelFormat::Gray8, 1, 1, true, 1.0).unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)));
    }

    #[test]
    fn encode_rejects_bad_scale() {
        let img = float_image(1, 1, 1);
        assert!(encode_pfm(&img, true, 0.0).is_err());
        assert!(encode_pfm(&img, true, f32::INFINITY).is_err());
    }

    #[test]
    fn apply_pfm_scale_multiplies_every_sample() {
        let mut img = float_image(3, 2, 3);
        // Snapshot the unscaled samples so we can compare against ×2.5.
        let before: Vec<f32> = img.planes[0]
            .data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        apply_pfm_scale(&mut img, 2.5).unwrap();
        for (i, c) in img.planes[0].data.chunks_exact(4).enumerate() {
            let got = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            assert_eq!(got, before[i] * 2.5, "sample {i}");
        }
    }

    #[test]
    fn apply_pfm_scale_unit_is_noop() {
        let mut img = float_image(2, 2, 1);
        let original = img.planes[0].data.clone();
        apply_pfm_scale(&mut img, 1.0).unwrap();
        assert_eq!(img.planes[0].data, original);
    }

    #[test]
    fn apply_pfm_scale_rejects_non_float_and_non_finite() {
        let mut gray8 = PbmImage {
            width: 1,
            height: 1,
            pixel_format: PbmPixelFormat::Gray8,
            planes: vec![PbmPlane {
                stride: 1,
                data: vec![0u8],
            }],
            pts: None,
        };
        assert!(matches!(
            apply_pfm_scale(&mut gray8, 2.0),
            Err(Error::Unsupported(_))
        ));
        let mut img = float_image(1, 1, 1);
        assert!(apply_pfm_scale(&mut img, f32::NAN).is_err());
        assert!(apply_pfm_scale(&mut img, f32::INFINITY).is_err());
    }

    #[test]
    fn apply_inverse_pfm_scale_divides_every_sample() {
        let mut img = float_image(3, 2, 3);
        let before: Vec<f32> = img.planes[0]
            .data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        apply_inverse_pfm_scale(&mut img, 2.5).unwrap();
        for (i, c) in img.planes[0].data.chunks_exact(4).enumerate() {
            let got = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            assert_eq!(got, before[i] / 2.5, "sample {i}");
        }
    }

    #[test]
    fn apply_inverse_pfm_scale_unit_is_noop() {
        let mut img = float_image(2, 2, 1);
        let original = img.planes[0].data.clone();
        apply_inverse_pfm_scale(&mut img, 1.0).unwrap();
        assert_eq!(img.planes[0].data, original);
    }

    #[test]
    fn apply_inverse_pfm_scale_rejects_non_float_zero_and_non_finite() {
        let mut gray8 = PbmImage {
            width: 1,
            height: 1,
            pixel_format: PbmPixelFormat::Gray8,
            planes: vec![PbmPlane {
                stride: 1,
                data: vec![0u8],
            }],
            pts: None,
        };
        assert!(matches!(
            apply_inverse_pfm_scale(&mut gray8, 2.0),
            Err(Error::Unsupported(_))
        ));
        let mut img = float_image(1, 1, 1);
        assert!(apply_inverse_pfm_scale(&mut img, f32::NAN).is_err());
        assert!(apply_inverse_pfm_scale(&mut img, f32::INFINITY).is_err());
        assert!(apply_inverse_pfm_scale(&mut img, 0.0).is_err());
    }

    #[test]
    fn apply_inverse_then_apply_round_trips() {
        // Dividing by a factor then multiplying by the same factor must
        // return the original samples (the values were chosen so the
        // /2.0 then *2.0 round-trip is exact in f32).
        let mut img = float_image(4, 3, 3);
        let original = img.planes[0].data.clone();
        apply_inverse_pfm_scale(&mut img, 2.0).unwrap();
        apply_pfm_scale(&mut img, 2.0).unwrap();
        assert_eq!(img.planes[0].data, original);
    }

    #[test]
    fn encode_pfm_scaled_unit_matches_plain_encode() {
        // scale 1.0 must be byte-identical to encode_pfm(.., 1.0).
        let img = float_image(5, 4, 3);
        let plain = encode_pfm(&img, true, 1.0).unwrap();
        let scaled = encode_pfm_scaled(&img, true, 1.0).unwrap();
        assert_eq!(plain, scaled);
    }

    #[test]
    fn encode_pfm_scaled_does_not_mutate_input() {
        let img = float_image(3, 3, 1);
        let snapshot = img.planes[0].data.clone();
        let _ = encode_pfm_scaled(&img, false, 4.0).unwrap();
        assert_eq!(img.planes[0].data, snapshot);
    }

    #[test]
    fn encode_pfm_scaled_records_factor_in_header() {
        let img = float_image(2, 2, 1);
        let bytes = encode_pfm_scaled(&img, false, 2.0).unwrap();
        // Big-endian => positive scale line.
        assert!(bytes.starts_with(b"Pf\n2 2\n2.0\n"), "header sets factor");
        let (_img, info) = decode_pfm(&bytes).unwrap();
        assert_eq!(info.scale, 2.0);
        assert!(!info.little_endian);
    }

    #[test]
    fn encode_pfm_scaled_then_decode_scaled_recovers_linear_values() {
        // The headline contract: treat the image samples as linear values
        // L, store them folded by /scale, and confirm a reader that
        // applies the factor recovers L (within f32 precision). A factor
        // of 2.0 keeps the /2 then *2 round-trip exact for these samples.
        let img = float_image(4, 3, 3);
        let linear: Vec<f32> = img.planes[0]
            .data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let bytes = encode_pfm_scaled(&img, true, 2.0).unwrap();
        // On disk the samples are L/2; reading-with-scale multiplies back.
        let (raw, _) = decode_pfm(&bytes).unwrap();
        for (i, c) in raw.planes[0].data.chunks_exact(4).enumerate() {
            let stored = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            assert_eq!(stored, linear[i] / 2.0, "stored sample {i}");
        }
        let (scaled, info) = decode_pfm_scaled(&bytes).unwrap();
        assert_eq!(info.scale, 2.0);
        for (i, c) in scaled.planes[0].data.chunks_exact(4).enumerate() {
            let recovered = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            assert_eq!(recovered, linear[i], "recovered linear sample {i}");
        }
    }

    #[test]
    fn encode_pfm_scaled_rejects_non_float_and_bad_scale() {
        let gray8 = PbmImage {
            width: 1,
            height: 1,
            pixel_format: PbmPixelFormat::Gray8,
            planes: vec![PbmPlane {
                stride: 1,
                data: vec![0u8],
            }],
            pts: None,
        };
        assert!(matches!(
            encode_pfm_scaled(&gray8, true, 1.0),
            Err(Error::Unsupported(_))
        ));
        let img = float_image(1, 1, 1);
        assert!(encode_pfm_scaled(&img, true, 0.0).is_err());
        assert!(encode_pfm_scaled(&img, true, f32::INFINITY).is_err());
        assert!(encode_pfm_scaled(&img, true, f32::NAN).is_err());
    }

    #[test]
    fn decode_pfm_scaled_applies_header_factor() {
        // Encode at scale 3.0 (big-endian), then decode-with-scale and
        // confirm every sample is the raw value × 3.0 while the reported
        // scale is still the header's original 3.0.
        let img = float_image(4, 3, 1);
        let bytes = encode_pfm(&img, false, 3.0).unwrap();
        let (raw, _) = decode_pfm(&bytes).unwrap();
        let (scaled, info) = decode_pfm_scaled(&bytes).unwrap();
        assert_eq!(info.scale, 3.0);
        for (rc, sc) in raw.planes[0]
            .data
            .chunks_exact(4)
            .zip(scaled.planes[0].data.chunks_exact(4))
        {
            let r = f32::from_le_bytes([rc[0], rc[1], rc[2], rc[3]]);
            let s = f32::from_le_bytes([sc[0], sc[1], sc[2], sc[3]]);
            assert_eq!(s, r * 3.0);
        }
    }

    #[test]
    fn scaled_then_reencode_unit_reproduces_linear_values() {
        // decode_pfm_scaled folds the factor into the samples, so
        // re-encoding with scale 1.0 round-trips the scaled linear values.
        let img = float_image(3, 3, 3);
        let bytes = encode_pfm(&img, true, 2.0).unwrap();
        let (scaled, _) = decode_pfm_scaled(&bytes).unwrap();
        let reencoded = encode_pfm(&scaled, true, 1.0).unwrap();
        let (back, info) = decode_pfm(&reencoded).unwrap();
        assert_eq!(info.scale, 1.0);
        assert_eq!(back.planes[0].data, scaled.planes[0].data);
    }

    #[test]
    fn consumed_count_is_header_plus_raster() {
        // A 5×4 grayscale PFM: raster = 5*4*1*4 = 80 bytes; the consumed
        // count must be the header length + 80, i.e. the whole encoding.
        let img = float_image(5, 4, 1);
        let bytes = encode_pfm(&img, true, 1.0).unwrap();
        let (_image, info, consumed) = decode_pfm_consumed(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(consumed - 80, b"Pf\n5 4\n-1.0\n".len());
        assert_eq!(info.channels, 1);
    }

    #[test]
    fn consumed_count_rgb_accounts_for_three_channels() {
        let img = float_image(3, 2, 3);
        let bytes = encode_pfm(&img, false, 1.0).unwrap();
        let (_image, info, consumed) = decode_pfm_consumed(&bytes).unwrap();
        // Raster = 3*2*3*4 = 72 bytes.
        assert_eq!(consumed, bytes.len());
        assert_eq!(consumed - 72, b"PF\n3 2\n1.0\n".len());
        assert_eq!(info.channels, 3);
    }

    #[test]
    fn consumed_walks_concatenated_pfm_stream_preserving_metadata() {
        // Pack two distinct PFM images back to back (different byte order,
        // scale, channels and size) and walk them by cursor, confirming
        // each image's PfmHeaderInfo survives the walk.
        let a = float_image(4, 3, 1);
        let b = float_image(2, 2, 3);
        let mut stream = encode_pfm(&a, true, 1.0).unwrap();
        stream.extend(encode_pfm(&b, false, 2.5).unwrap());

        let mut off = 0;
        let mut seen = Vec::new();
        while off < stream.len() {
            let (image, info, consumed) = decode_pfm_consumed(&stream[off..]).unwrap();
            assert!(consumed > 0);
            seen.push((info, image.width, image.height, image.pixel_format));
            off += consumed;
        }
        assert_eq!(off, stream.len());
        assert_eq!(seen.len(), 2);

        let (info0, w0, h0, fmt0) = seen[0];
        assert!(info0.little_endian);
        assert_eq!(info0.scale, 1.0);
        assert_eq!(info0.channels, 1);
        assert_eq!((w0, h0), (4, 3));
        assert_eq!(fmt0, PbmPixelFormat::GrayF32);

        let (info1, w1, h1, fmt1) = seen[1];
        assert!(!info1.little_endian);
        assert_eq!(info1.scale, 2.5);
        assert_eq!(info1.channels, 3);
        assert_eq!((w1, h1), (2, 2));
        assert_eq!(fmt1, PbmPixelFormat::RgbF32);
    }

    #[test]
    fn consumed_matches_decode_pfm_image_bytes() {
        // The pixels recovered by decode_pfm_consumed must equal those from
        // the plain decode_pfm path — only the extra byte count differs.
        let img = float_image(6, 5, 3);
        let bytes = encode_pfm(&img, false, 3.0).unwrap();
        let (plain, plain_info) = decode_pfm(&bytes).unwrap();
        let (rich, rich_info, consumed) = decode_pfm_consumed(&bytes).unwrap();
        assert_eq!(plain.planes[0].data, rich.planes[0].data);
        assert_eq!(plain_info, rich_info);
        assert_eq!(consumed, bytes.len());
    }

    #[test]
    fn consumed_rejects_non_pfm_input() {
        // A P5 (binary PGM) header is a valid Netpbm magic but not PFM, so
        // the PFM-specific entry must reject it rather than mis-walk it.
        let buf = b"P5\n2 2\n255\n\x00\x01\x02\x03";
        assert!(decode_pfm_consumed(buf).is_err());
    }

    #[test]
    fn swap_bytes_u32_row_swaps_every_sample() {
        // Four samples covering the full byte range.
        let src: [u8; 16] = [
            0x12, 0x34, 0x56, 0x78, // sample 0
            0xff, 0x00, 0xa5, 0x5a, // sample 1
            0xde, 0xad, 0xbe, 0xef, // sample 2
            0x00, 0x11, 0x22, 0x33, // sample 3
        ];
        let mut dst = [0u8; 16];
        swap_bytes_u32_row(&src, &mut dst);
        assert_eq!(
            dst,
            [
                0x78, 0x56, 0x34, 0x12, // sample 0 reversed
                0x5a, 0xa5, 0x00, 0xff, // sample 1 reversed
                0xef, 0xbe, 0xad, 0xde, // sample 2 reversed
                0x33, 0x22, 0x11, 0x00, // sample 3 reversed
            ]
        );
    }

    #[test]
    fn swap_bytes_u32_row_is_self_inverse() {
        let src: [u8; 12] = [
            0xaa, 0xbb, 0xcc, 0xdd, 0x01, 0x02, 0x03, 0x04, 0xfe, 0xed, 0xfa, 0xce,
        ];
        let mut once = [0u8; 12];
        swap_bytes_u32_row(&src, &mut once);
        let mut twice = [0u8; 12];
        swap_bytes_u32_row(&once, &mut twice);
        assert_eq!(twice, src);
    }
}
