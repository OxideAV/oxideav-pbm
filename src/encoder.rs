//! Netpbm encoder.
//!
//! Picks an output magic from the input [`PbmPixelFormat`]:
//!
//! | PbmPixelFormat     | Output |
//! |--------------------|--------|
//! | `MonoBlack`        | P4     |
//! | `Gray8`            | P5 (maxval 255) |
//! | `Gray16Le`         | P5 (maxval 65535, big-endian samples on disk) |
//! | `Rgb24`            | P6 (maxval 255) |
//! | `Rgb48Le`          | P6 (maxval 65535) |
//! | `Rgba` / `Bgra`    | P7 RGB_ALPHA (maxval 255) |
//! | `Rgba64Le`         | P7 RGB_ALPHA (maxval 65535) |
//! | `Ya8`              | P7 GRAYSCALE_ALPHA (maxval 255) |
//!
//! Other pixel formats are rejected so the caller gets a clear error
//! instead of a silent conversion. ASCII output (P1/P2/P3) can be
//! requested via [`encode_pbm_ascii`] — the binary path is always
//! preferred for size.
//!
//! Callers that need to pin the on-disk magic explicitly (regardless of
//! the input [`PbmPixelFormat`]) use [`encode_pbm_with_format`] +
//! [`PbmEncodeFormat`] — useful when the consumer cares whether they
//! get the plain ASCII form (`P1`/`P2`/`P3`) or the binary form
//! (`P4`/`P5`/`P6`/`P7`).

use crate::error::{PbmError as Error, Result};

use crate::ascii::{encode_ascii_body_bits, encode_ascii_body_u8};
use crate::binary::{encode_p4_body, swap_bytes_u16_row};
use crate::image::{PbmImage, PbmPixelFormat, PbmPlane};

#[cfg(feature = "registry")]
use oxideav_core::Encoder;
#[cfg(feature = "registry")]
use oxideav_core::{CodecId, CodecParameters, Frame, Packet, TimeBase};

#[cfg(feature = "registry")]
pub fn make_encoder(params: &CodecParameters) -> oxideav_core::Result<Box<dyn Encoder>> {
    let mut out_params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    out_params.width = params.width;
    out_params.height = params.height;
    out_params.pixel_format = params.pixel_format;
    Ok(Box::new(PbmEncoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        out_params,
        pending: None,
        eof: false,
    }))
}

#[cfg(feature = "registry")]
struct PbmEncoder {
    codec_id: CodecId,
    out_params: CodecParameters,
    pending: Option<Vec<u8>>,
    eof: bool,
}

#[cfg(feature = "registry")]
impl Encoder for PbmEncoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn output_params(&self) -> &CodecParameters {
        &self.out_params
    }
    fn send_frame(&mut self, frame: &Frame) -> oxideav_core::Result<()> {
        let vf = match frame {
            Frame::Video(v) => v,
            _ => {
                return Err(oxideav_core::Error::invalid(
                    "PBM encoder: expected video frame",
                ))
            }
        };
        let format = self.out_params.pixel_format.ok_or_else(|| {
            oxideav_core::Error::invalid("PBM encoder: pixel_format missing in CodecParameters")
        })?;
        let width = self.out_params.width.ok_or_else(|| {
            oxideav_core::Error::invalid("PBM encoder: width missing in CodecParameters")
        })?;
        let height = self.out_params.height.ok_or_else(|| {
            oxideav_core::Error::invalid("PBM encoder: height missing in CodecParameters")
        })?;
        let pbm_format = crate::registry::pixel_format_to_pbm(format).ok_or_else(|| {
            oxideav_core::Error::invalid(format!(
                "PBM encoder: pixel format {format:?} not representable as Netpbm"
            ))
        })?;
        if vf.planes.is_empty() {
            return Err(oxideav_core::Error::invalid("PBM encoder: empty plane"));
        }
        let plane = PbmPlane {
            stride: vf.planes[0].stride,
            data: vf.planes[0].data.clone(),
        };
        let bytes = encode_pbm_plane(&plane, pbm_format, width, height)?;
        self.pending = Some(bytes);
        Ok(())
    }
    fn receive_packet(&mut self) -> oxideav_core::Result<Packet> {
        match self.pending.take() {
            Some(bytes) => {
                let mut pkt = Packet::new(0, TimeBase::new(1, 1), bytes);
                pkt.flags.keyframe = true;
                Ok(pkt)
            }
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

/// Encode a [`PbmImage`] into the closest matching binary Netpbm
/// variant.
pub fn encode_pbm(image: &PbmImage) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("PBM encoder: empty plane"));
    }
    encode_pbm_plane(
        &image.planes[0],
        image.pixel_format,
        image.width,
        image.height,
    )
}

/// Output-format selector for [`encode_pbm_with_format`].
///
/// Encoders sometimes need to pin the on-disk magic — for instance, a
/// downstream tool that only reads `pamfile`-style PAM, or a debugging
/// dump that wants the plain-ASCII PNM form.
///
/// `Auto*` modes ask the encoder to pick the closest matching magic
/// from the [`PbmPixelFormat`] (same behaviour as [`encode_pbm`] /
/// [`encode_pbm_ascii`]). Explicit modes (`Pnm1` … `Pam7`) force a
/// specific magic; the encoder still returns `Unsupported` if the
/// input pixel format cannot be represented in that magic (e.g. P1
/// only accepts `MonoBlack`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PbmEncodeFormat {
    /// Pick the closest binary magic (P4/P5/P6/P7) — same as
    /// [`encode_pbm`].
    AutoBinary,
    /// Pick the closest plain-ASCII magic (P1/P2/P3) — same as
    /// [`encode_pbm_ascii`]. Errors on alpha pixel formats since
    /// P1/P2/P3 cannot represent them.
    AutoAscii,
    /// Force `P1` plain-ASCII bitmap. Only valid for `MonoBlack`.
    Pnm1,
    /// Force `P2` plain-ASCII graymap. Valid for `Gray8` (always emits
    /// MAXVAL 255 — 16-bit grayscale is rejected since ASCII PGM has
    /// no defined upper-bound representation in our `u16` body
    /// encoder).
    Pnm2,
    /// Force `P3` plain-ASCII pixmap. Valid for `Rgb24` (MAXVAL 255).
    Pnm3,
    /// Force `P4` binary bitmap. Only valid for `MonoBlack`.
    Pnm4,
    /// Force `P5` binary graymap. Valid for `Gray8` (MAXVAL 255) and
    /// `Gray16Le` (MAXVAL 65535, big-endian samples on disk).
    Pnm5,
    /// Force `P6` binary pixmap. Valid for `Rgb24` (MAXVAL 255) and
    /// `Rgb48Le` (MAXVAL 65535).
    Pnm6,
    /// Force `P7` PAM. Valid for every supported [`PbmPixelFormat`]
    /// except `MonoBlack` (where P4 is the natural form — PAM
    /// `BLACKANDWHITE` is supported via the auto path on decode but
    /// the encoder always emits P4 for `MonoBlack` since P7
    /// `BLACKANDWHITE` would be a bigger header for the same payload).
    Pam7,
    /// Force Portable FloatMap (`Pf` for `GrayF32`, `PF` for `RgbF32`).
    /// Only valid for the two float pixel formats; emits little-endian
    /// samples with a unit scale. Callers needing an explicit byte order
    /// or scale use [`crate::pfm::encode_pfm`] directly.
    Pfm,
}

/// Encode a [`PbmImage`] with an explicit choice of output magic.
///
/// `Auto*` variants delegate to [`encode_pbm`] / [`encode_pbm_ascii`].
/// Explicit `Pnm*` / `Pam7` variants force the specified magic and
/// reject pixel formats that can't be represented in it.
pub fn encode_pbm_with_format(image: &PbmImage, format: PbmEncodeFormat) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("PBM encoder: empty plane"));
    }
    let plane = &image.planes[0];
    let w = image.width as usize;
    let h = image.height as usize;
    if plane.data.len() < plane.stride * h {
        return Err(Error::invalid("PBM encoder: plane truncated"));
    }
    match format {
        PbmEncodeFormat::AutoBinary => encode_pbm(image),
        PbmEncodeFormat::AutoAscii => encode_pbm_ascii(image),
        PbmEncodeFormat::Pnm1 => match image.pixel_format {
            PbmPixelFormat::MonoBlack => Ok(emit_ascii_pbm_header_and_body(plane, w, h)),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P1"
            ))),
        },
        PbmEncodeFormat::Pnm2 => match image.pixel_format {
            PbmPixelFormat::Gray8 => Ok(emit_ascii_pgm_8(plane, w, h)),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P2"
            ))),
        },
        PbmEncodeFormat::Pnm3 => match image.pixel_format {
            PbmPixelFormat::Rgb24 => Ok(emit_ascii_ppm_8(plane, w, h)),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P3"
            ))),
        },
        PbmEncodeFormat::Pnm4 => match image.pixel_format {
            PbmPixelFormat::MonoBlack => encode_p4(plane, w, h),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P4"
            ))),
        },
        PbmEncodeFormat::Pnm5 => match image.pixel_format {
            PbmPixelFormat::Gray8 => encode_p5_gray8(plane, w, h),
            PbmPixelFormat::Gray16Le => encode_p5_gray16(plane, w, h),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P5"
            ))),
        },
        PbmEncodeFormat::Pnm6 => match image.pixel_format {
            PbmPixelFormat::Rgb24 => encode_p6_rgb8(plane, w, h),
            PbmPixelFormat::Rgb48Le => encode_p6_rgb16(plane, w, h),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P6"
            ))),
        },
        PbmEncodeFormat::Pam7 => match image.pixel_format {
            PbmPixelFormat::Gray8 => encode_p7_gray8(plane, w, h),
            PbmPixelFormat::Gray16Le => encode_p7_gray16(plane, w, h),
            PbmPixelFormat::Rgb24 => encode_p7_rgb8(plane, w, h),
            PbmPixelFormat::Rgb48Le => encode_p7_rgb16(plane, w, h),
            PbmPixelFormat::Rgba => encode_p7_rgba8(plane, w, h),
            PbmPixelFormat::Bgra => encode_p7_bgra8(plane, w, h),
            PbmPixelFormat::Rgba64Le => encode_p7_rgba16(plane, w, h),
            PbmPixelFormat::Ya8 => encode_p7_ya8(plane, w, h),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P7"
            ))),
        },
        PbmEncodeFormat::Pfm => match image.pixel_format {
            PbmPixelFormat::GrayF32 | PbmPixelFormat::RgbF32 => crate::pfm::encode_pfm_plane(
                plane,
                image.pixel_format,
                image.width,
                image.height,
                true,
                1.0,
            ),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as a Portable FloatMap"
            ))),
        },
    }
}

/// Encode a single [`PbmPlane`] (width × height pixels in `format`)
/// into a binary Netpbm file. Lower-level than [`encode_pbm`] for
/// callers that already have plane bytes laid out without a wrapping
/// [`PbmImage`].
pub fn encode_pbm_plane(
    plane: &PbmPlane,
    format: PbmPixelFormat,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    if plane.data.len() < plane.stride * h {
        return Err(Error::invalid("PBM encoder: plane truncated"));
    }
    match format {
        PbmPixelFormat::MonoBlack => encode_p4(plane, w, h),
        PbmPixelFormat::Gray8 => encode_p5_gray8(plane, w, h),
        PbmPixelFormat::Gray16Le => encode_p5_gray16(plane, w, h),
        PbmPixelFormat::Rgb24 => encode_p6_rgb8(plane, w, h),
        PbmPixelFormat::Rgb48Le => encode_p6_rgb16(plane, w, h),
        PbmPixelFormat::Rgba => encode_p7_rgba8(plane, w, h),
        PbmPixelFormat::Bgra => encode_p7_bgra8(plane, w, h),
        PbmPixelFormat::Rgba64Le => encode_p7_rgba16(plane, w, h),
        PbmPixelFormat::Ya8 => encode_p7_ya8(plane, w, h),
        // Float maps have no integer Netpbm form — emit Portable
        // FloatMap (`Pf` / `PF`). Default to little-endian (no byte swap
        // from the little-endian in-memory plane) with a unit scale.
        PbmPixelFormat::GrayF32 | PbmPixelFormat::RgbF32 => {
            crate::pfm::encode_pfm_plane(plane, format, width, height, true, 1.0)
        }
    }
}

/// ASCII variant: emit P1/P2/P3 from a [`PbmImage`]. Less efficient
/// (≥ 3× larger on disk) but the man pages still document the plain
/// forms and some tools require them.
pub fn encode_pbm_ascii(image: &PbmImage) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("PBM ASCII encoder: empty plane"));
    }
    encode_pbm_ascii_plane(
        &image.planes[0],
        image.pixel_format,
        image.width,
        image.height,
    )
}

/// ASCII variant: emit P1/P2/P3 from a single plane.
pub fn encode_pbm_ascii_plane(
    plane: &PbmPlane,
    format: PbmPixelFormat,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    match format {
        PbmPixelFormat::MonoBlack => Ok(emit_ascii_pbm_header_and_body(plane, w, h)),
        PbmPixelFormat::Gray8 => Ok(emit_ascii_pgm_8(plane, w, h)),
        PbmPixelFormat::Rgb24 => Ok(emit_ascii_ppm_8(plane, w, h)),
        other => Err(Error::unsupported(format!(
            "PBM ASCII encoder: pixel format {other:?} not supported"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Binary writers — one per output magic
// ---------------------------------------------------------------------------

fn header_pnm(magic: u8, w: usize, h: usize, maxval: Option<u32>) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.push(b'P');
    out.push(magic);
    out.push(b'\n');
    out.extend_from_slice(format!("{w} {h}\n").as_bytes());
    if let Some(mv) = maxval {
        out.extend_from_slice(format!("{mv}\n").as_bytes());
    }
    out
}

fn encode_p4(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    // Input is `MonoBlack`: MSB-first packed bits, 1 = black, rows
    // padded to a byte. P4's wire format is identical, but we may have
    // an input stride larger than the spec's row_bytes — repack just
    // in case.
    let row_bytes = w.div_ceil(8);
    let mut bits = vec![0u8; w * h];
    for y in 0..h {
        let row = &plane.data[y * plane.stride..y * plane.stride + row_bytes];
        for x in 0..w {
            let bit = (row[x / 8] >> (7 - (x % 8))) & 1;
            bits[y * w + x] = bit;
        }
    }
    let body = encode_p4_body(w as u32, h as u32, &bits);
    let mut out = header_pnm(b'4', w, h, None);
    out.extend(body);
    Ok(out)
}

fn encode_p5_gray8(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pnm(b'5', w, h, Some(255));
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w]);
    }
    Ok(out)
}

fn encode_p5_gray16(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pnm(b'5', w, h, Some(65535));
    // `Gray16Le` stores LE bytes; on-disk Netpbm wants BE. Funnel the
    // per-row LE→BE swap through the row-level `swap_bytes_u16_row`
    // helper from `binary.rs` so the inner loop walks
    // `chunks_exact(2)` over a pre-sized `&mut [u8]` destination and
    // lowers to a vectorised swap (`REV16.16B` on aarch64, `pshufb` /
    // `vpshufb` on x86). Same shape as the round-205 PFM 32-bit helper.
    let row_bytes = w * 2;
    let body_start = out.len();
    out.resize(body_start + row_bytes * h, 0);
    for y in 0..h {
        let src = &plane.data[y * plane.stride..y * plane.stride + row_bytes];
        let dst = &mut out[body_start + y * row_bytes..body_start + (y + 1) * row_bytes];
        swap_bytes_u16_row(src, dst);
    }
    Ok(out)
}

fn encode_p6_rgb8(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pnm(b'6', w, h, Some(255));
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w * 3]);
    }
    Ok(out)
}

fn encode_p6_rgb16(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pnm(b'6', w, h, Some(65535));
    // Same LE→BE row swap as P5 16-bit, but three samples per pixel.
    // The chunked swap is channel-agnostic (it just walks 2-byte
    // samples), so 3 channels reuses the helper unchanged.
    let row_bytes = w * 6;
    let body_start = out.len();
    out.resize(body_start + row_bytes * h, 0);
    for y in 0..h {
        let src = &plane.data[y * plane.stride..y * plane.stride + row_bytes];
        let dst = &mut out[body_start + y * row_bytes..body_start + (y + 1) * row_bytes];
        swap_bytes_u16_row(src, dst);
    }
    Ok(out)
}

fn header_pam(w: usize, h: usize, depth: u32, maxval: u32, tupltype: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(b"P7\n");
    out.extend_from_slice(format!("WIDTH {w}\n").as_bytes());
    out.extend_from_slice(format!("HEIGHT {h}\n").as_bytes());
    out.extend_from_slice(format!("DEPTH {depth}\n").as_bytes());
    out.extend_from_slice(format!("MAXVAL {maxval}\n").as_bytes());
    out.extend_from_slice(format!("TUPLTYPE {tupltype}\n").as_bytes());
    out.extend_from_slice(b"ENDHDR\n");
    out
}

fn encode_p7_gray8(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 1, 255, "GRAYSCALE");
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w]);
    }
    Ok(out)
}

fn encode_p7_gray16(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 1, 65535, "GRAYSCALE");
    for y in 0..h {
        let row = &plane.data[y * plane.stride..y * plane.stride + w * 2];
        // `Gray16Le` is LE in memory; on-disk PAM wants big-endian.
        for chunk in row.chunks_exact(2) {
            out.push(chunk[1]);
            out.push(chunk[0]);
        }
    }
    Ok(out)
}

fn encode_p7_rgb8(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 3, 255, "RGB");
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w * 3]);
    }
    Ok(out)
}

fn encode_p7_rgb16(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 3, 65535, "RGB");
    // Identical body shape to P6 16-bit (PAM with `RGB` tupltype is the
    // same row-major three-sample layout); reuse the row-level swap.
    let row_bytes = w * 6;
    let body_start = out.len();
    out.resize(body_start + row_bytes * h, 0);
    for y in 0..h {
        let src = &plane.data[y * plane.stride..y * plane.stride + row_bytes];
        let dst = &mut out[body_start + y * row_bytes..body_start + (y + 1) * row_bytes];
        swap_bytes_u16_row(src, dst);
    }
    Ok(out)
}

fn encode_p7_rgba8(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 4, 255, "RGB_ALPHA");
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w * 4]);
    }
    Ok(out)
}

fn encode_p7_bgra8(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    // Reorder BGRA → RGBA on the way out so the file declares RGB_ALPHA
    // and any decoder reads them back as such.
    let mut out = header_pam(w, h, 4, 255, "RGB_ALPHA");
    for y in 0..h {
        let row = &plane.data[y * plane.stride..y * plane.stride + w * 4];
        for px in row.chunks_exact(4) {
            out.push(px[2]);
            out.push(px[1]);
            out.push(px[0]);
            out.push(px[3]);
        }
    }
    Ok(out)
}

fn encode_p7_rgba16(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 4, 65535, "RGB_ALPHA");
    // Four 16-bit channels per pixel (R/G/B/A); the row-level swap is
    // channel-agnostic so we reuse the same helper.
    let row_bytes = w * 8;
    let body_start = out.len();
    out.resize(body_start + row_bytes * h, 0);
    for y in 0..h {
        let src = &plane.data[y * plane.stride..y * plane.stride + row_bytes];
        let dst = &mut out[body_start + y * row_bytes..body_start + (y + 1) * row_bytes];
        swap_bytes_u16_row(src, dst);
    }
    Ok(out)
}

fn encode_p7_ya8(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 2, 255, "GRAYSCALE_ALPHA");
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w * 2]);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// ASCII writers (P1/P2/P3)
// ---------------------------------------------------------------------------

fn emit_ascii_pbm_header_and_body(plane: &PbmPlane, w: usize, h: usize) -> Vec<u8> {
    // Direct bit-to-ASCII writer — avoids the temporary `Vec<u16>` the
    // generic `encode_ascii_body` would otherwise need.
    let mut out = header_pnm(b'1', w, h, None);
    out.extend(encode_ascii_body_bits(&plane.data, plane.stride, w, h));
    out
}

fn emit_ascii_pgm_8(plane: &PbmPlane, w: usize, h: usize) -> Vec<u8> {
    // P2 / Gray8: samples already fit in u8; route through the
    // u8-specialised writer instead of widening to `Vec<u16>` first.
    let mut samples: Vec<u8> = Vec::with_capacity(w * h);
    for y in 0..h {
        samples.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w]);
    }
    let mut out = header_pnm(b'2', w, h, Some(255));
    out.extend(encode_ascii_body_u8(&samples, w));
    out
}

fn emit_ascii_ppm_8(plane: &PbmPlane, w: usize, h: usize) -> Vec<u8> {
    // P3 / Rgb24: same idea as P2 — samples are u8 already, so the
    // u8-specialised writer skips the `Vec<u16>` widen step. The
    // column-stride is `w * 3` (three samples per pixel).
    let mut samples: Vec<u8> = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        samples.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w * 3]);
    }
    let mut out = header_pnm(b'3', w, h, Some(255));
    out.extend(encode_ascii_body_u8(&samples, w * 3));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_image(
        format: PbmPixelFormat,
        w: u32,
        h: u32,
        stride: usize,
        data: Vec<u8>,
    ) -> PbmImage {
        PbmImage {
            width: w,
            height: h,
            pixel_format: format,
            planes: vec![PbmPlane { stride, data }],
            pts: None,
        }
    }

    #[test]
    fn encode_p6_rgb8_smoke() {
        let img = make_image(
            PbmPixelFormat::Rgb24,
            2,
            2,
            6,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        );
        let bytes = encode_pbm(&img).unwrap();
        assert!(bytes.starts_with(b"P6\n2 2\n255\n"));
        let body = &bytes[bytes.iter().position(|&b| b == b'\n').unwrap() + 1..];
        let body = &body[body.iter().position(|&b| b == b'\n').unwrap() + 1..];
        let body = &body[body.iter().position(|&b| b == b'\n').unwrap() + 1..];
        assert_eq!(body, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }

    #[test]
    fn encode_p5_gray16_swaps_to_be() {
        let img = make_image(
            PbmPixelFormat::Gray16Le,
            2,
            1,
            4,
            // LE input: 0x1234 then 0x5678
            vec![0x34, 0x12, 0x78, 0x56],
        );
        let bytes = encode_pbm(&img).unwrap();
        assert!(bytes.starts_with(b"P5\n2 1\n65535\n"));
        // Last 4 bytes = BE samples
        assert_eq!(&bytes[bytes.len() - 4..], &[0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn encode_p7_rgba_emits_rgb_alpha_tupltype() {
        let img = make_image(PbmPixelFormat::Rgba, 1, 1, 4, vec![10, 20, 30, 40]);
        let bytes = encode_pbm(&img).unwrap();
        assert!(bytes.starts_with(b"P7\n"));
        let s = std::str::from_utf8(&bytes[..bytes.len() - 4]).unwrap();
        assert!(s.contains("TUPLTYPE RGB_ALPHA"));
        assert_eq!(&bytes[bytes.len() - 4..], &[10, 20, 30, 40]);
    }

    #[test]
    fn explicit_format_p1_rejects_non_mono() {
        let img = make_image(PbmPixelFormat::Gray8, 1, 1, 1, vec![128]);
        let err = encode_pbm_with_format(&img, PbmEncodeFormat::Pnm1).unwrap_err();
        match err {
            Error::Unsupported(_) => {}
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn explicit_format_pam7_for_gray8_emits_p7_grayscale() {
        let img = make_image(PbmPixelFormat::Gray8, 2, 1, 2, vec![10, 20]);
        let bytes = encode_pbm_with_format(&img, PbmEncodeFormat::Pam7).unwrap();
        assert!(bytes.starts_with(b"P7\n"));
        let s = std::str::from_utf8(&bytes[..bytes.len() - 2]).unwrap();
        assert!(s.contains("TUPLTYPE GRAYSCALE"));
        assert!(s.contains("DEPTH 1"));
        assert!(s.contains("MAXVAL 255"));
        assert_eq!(&bytes[bytes.len() - 2..], &[10, 20]);
    }

    #[test]
    fn explicit_format_pam7_rgb16_be_swap() {
        // Verify P7 RGB 16-bit BE swap is the same as P6 16-bit's.
        let img = make_image(
            PbmPixelFormat::Rgb48Le,
            1,
            1,
            6,
            // R=0x0102, G=0x0304, B=0x0506 in LE
            vec![0x02, 0x01, 0x04, 0x03, 0x06, 0x05],
        );
        let bytes = encode_pbm_with_format(&img, PbmEncodeFormat::Pam7).unwrap();
        let body = &bytes[bytes.len() - 6..];
        assert_eq!(body, &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    }

    #[test]
    fn explicit_format_p5_for_gray16_is_canonical() {
        // P5 with 16-bit gray should be the same as the auto-binary path
        // for `Gray16Le`.
        let img = make_image(
            PbmPixelFormat::Gray16Le,
            2,
            1,
            4,
            vec![0x34, 0x12, 0x78, 0x56],
        );
        let auto = encode_pbm(&img).unwrap();
        let explicit = encode_pbm_with_format(&img, PbmEncodeFormat::Pnm5).unwrap();
        assert_eq!(auto, explicit);
    }

    #[test]
    fn auto_ascii_routes_to_encode_pbm_ascii() {
        let img = make_image(PbmPixelFormat::Rgb24, 2, 1, 6, vec![1, 2, 3, 4, 5, 6]);
        let auto = encode_pbm_ascii(&img).unwrap();
        let explicit = encode_pbm_with_format(&img, PbmEncodeFormat::AutoAscii).unwrap();
        assert_eq!(auto, explicit);
    }
}
