//! Netpbm encoder.
//!
//! The natural (default) output magic per input [`PbmPixelFormat`]:
//!
//! | PbmPixelFormat          | Output |
//! |-------------------------|--------|
//! | `MonoBlack` / `MonoWhite` | P4 (bits inverted for `MonoWhite`) |
//! | `Gray8`                 | P5 (maxval 255) |
//! | `Gray16Le`              | P5 (maxval 65535, big-endian samples on disk) |
//! | `Rgb24`                 | P6 (maxval 255) |
//! | `Rgb48Le`               | P6 (maxval 65535) |
//! | `Rgba` / `Bgra`         | P7 RGB_ALPHA (maxval 255) |
//! | `Rgba64Le`              | P7 RGB_ALPHA (maxval 65535) |
//! | `Ya8`                   | P7 GRAYSCALE_ALPHA (maxval 255) |
//! | `Ya16Le`                | P7 GRAYSCALE_ALPHA (maxval 65535) |
//! | `GrayF32Le` / `RgbF32Le` | Portable FloatMap `Pf` / `PF` (little-endian, scale 1) |
//!
//! [`crate::EncodeOptions`] selects the other flavours — plain-text
//! P1 / P2 / P3 (`ascii`), the P7 PAM container for any integer layout
//! (`pam` / `tupltype`), a non-natural `MAXVAL` (`maxval`, samples
//! rescaled), and the PFM byte order / scale line. The contract entry
//! points ([`crate::encode`], [`crate::encode_rgb8`], …) live in
//! [`crate::api`] and call `encode_image`; the pre-contract
//! `encode_pbm*` functions and [`PbmEncodeFormat`] remain here as
//! deprecated wrappers over the same private writers.

use crate::error::{PbmError as Error, Result};

use crate::ascii::{encode_ascii_body, encode_ascii_body_bits, encode_ascii_body_u8};
use crate::binary::{bgra_to_rgba_row, copy_p4_row_msb, swap_bytes_u16_row};
use crate::header::Magic;
use crate::image::{PbmImage, PbmPixelFormat, Plane as PbmPlane};
use crate::options::EncodeOptions;

#[cfg(feature = "registry")]
use oxideav_core::Encoder;
#[cfg(feature = "registry")]
use oxideav_core::{CodecId, CodecParameters, Frame, Packet, TimeBase};

/// Factory registered with the codec registry. `params.options` is
/// parsed into [`EncodeOptions`] through the registry schema
/// (`ascii`, `pam`, `maxval`, `tupltype`, `pfm_little_endian`,
/// `pfm_scale`); an unknown key or malformed value is rejected here.
#[cfg(feature = "registry")]
pub fn make_encoder(params: &CodecParameters) -> oxideav_core::Result<Box<dyn Encoder>> {
    let mut out_params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    out_params.width = params.width;
    out_params.height = params.height;
    out_params.pixel_format = params.pixel_format;
    let options: EncodeOptions = oxideav_core::parse_options(&params.options)?;
    options.validate()?;
    Ok(Box::new(PbmEncoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        out_params,
        options,
        pending: None,
        eof: false,
    }))
}

#[cfg(feature = "registry")]
struct PbmEncoder {
    codec_id: CodecId,
    out_params: CodecParameters,
    options: EncodeOptions,
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
        // Thin adapter: rebuild the standalone image from the frame and
        // run the one standalone encoder (`crate::encode`).
        let image = PbmImage::from_video_frame(vf, &self.out_params)?;
        let bytes = encode_image(&image, &self.options)?;
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

// ---------------------------------------------------------------------------
// Contract encoder — the one implementation
// ---------------------------------------------------------------------------

/// Which body / container form the generic writer emits.
#[derive(Clone, Copy)]
enum Flavour<'a> {
    /// Plain-text P1 / P2 / P3.
    Ascii,
    /// Raw P4 / P5 / P6.
    Raw,
    /// P7 PAM with this `TUPLTYPE` token.
    Pam(&'a str),
}

/// The PAM `TUPLTYPE` the family defines for an integer layout.
fn standard_tupltype(fmt: PbmPixelFormat) -> &'static str {
    match fmt {
        PbmPixelFormat::MonoBlack | PbmPixelFormat::MonoWhite => "BLACKANDWHITE",
        PbmPixelFormat::Gray8 | PbmPixelFormat::Gray16Le => "GRAYSCALE",
        PbmPixelFormat::Ya8 | PbmPixelFormat::Ya16Le => "GRAYSCALE_ALPHA",
        PbmPixelFormat::Rgb24 | PbmPixelFormat::Rgb48Le => "RGB",
        PbmPixelFormat::Rgba | PbmPixelFormat::Bgra | PbmPixelFormat::Rgba64Le => "RGB_ALPHA",
        // Float layouts never reach the PAM writer.
        PbmPixelFormat::GrayF32Le | PbmPixelFormat::RgbF32Le => "RGB",
    }
}

/// Natural `MAXVAL` of an integer layout (1 for bilevel).
fn natural_maxval(fmt: PbmPixelFormat) -> u32 {
    match fmt.bits_per_channel() {
        1 => 1,
        8 => 255,
        _ => 65535,
    }
}

/// Invert a bilevel plane's bits (`MonoWhite` → the `MonoBlack` / P4
/// wire sense), row by row, keeping the stride.
fn invert_bilevel(plane: &PbmPlane, w: usize, h: usize) -> PbmPlane {
    let row_bytes = w.div_ceil(8);
    let mut data = vec![0u8; row_bytes * h];
    for y in 0..h {
        let src = &plane.data[y * plane.stride..y * plane.stride + row_bytes];
        for (d, s) in data[y * row_bytes..(y + 1) * row_bytes].iter_mut().zip(src) {
            *d = !*s;
        }
    }
    PbmPlane {
        stride: row_bytes,
        data,
    }
}

/// [`crate::encode`]: write `image` under `opts`. See the module docs
/// for the natural magic per layout and [`EncodeOptions`] for the
/// flavour knobs. Returns [`Error::Unsupported`] for combinations the
/// family cannot represent (alpha or float in plain text, PAM options
/// on a float layout, `ascii` + `pam`) and [`Error::InvalidData`] for an
/// image whose public fields were mutated into an inconsistent state.
pub(crate) fn encode_image(image: &PbmImage, opts: &EncodeOptions) -> Result<Vec<u8>> {
    opts.validate()?;
    image.validate()?;
    let fmt = image.format;
    let plane = &image.planes[0];
    let w = image.width as usize;
    let h = image.height as usize;

    if fmt.is_float() {
        if opts.ascii || opts.pam || opts.tupltype.is_some() || opts.maxval.is_some() {
            return Err(Error::unsupported(
                "Netpbm encoder: Portable FloatMap has no plain-text or PAM form and no MAXVAL",
            ));
        }
        return crate::pfm::encode_pfm_plane(
            plane,
            fmt,
            image.width,
            image.height,
            opts.pfm_little_endian,
            opts.pfm_scale,
        );
    }

    // Bilevel input is normalised to the wire sense (1 = black) first.
    let inverted;
    let plane = if fmt == PbmPixelFormat::MonoWhite {
        inverted = invert_bilevel(plane, w, h);
        &inverted
    } else {
        plane
    };
    let fmt = if fmt == PbmPixelFormat::MonoWhite {
        PbmPixelFormat::MonoBlack
    } else {
        fmt
    };

    let natural = natural_maxval(fmt);
    // Bilevel layouts have no MAXVAL knob (P1 / P4 carry none; PAM
    // BLACKANDWHITE is always 1).
    let maxval = if fmt.is_bilevel() {
        natural
    } else {
        opts.maxval.unwrap_or(natural)
    };
    let is_natural = maxval == natural;

    if opts.ascii {
        if fmt.has_alpha() {
            return Err(Error::unsupported(
                "Netpbm encoder: the plain-text forms P1 / P2 / P3 cannot carry alpha; use the raw PAM form",
            ));
        }
        if is_natural {
            return Ok(match fmt {
                PbmPixelFormat::MonoBlack => emit_ascii_pbm_header_and_body(plane, w, h),
                PbmPixelFormat::Gray8 => emit_ascii_pgm_8(plane, w, h),
                PbmPixelFormat::Gray16Le => emit_ascii_pgm_16(plane, w, h),
                PbmPixelFormat::Rgb24 => emit_ascii_ppm_8(plane, w, h),
                _ => emit_ascii_ppm_16(plane, w, h),
            });
        }
        return encode_generic(plane, fmt, w, h, maxval, Flavour::Ascii);
    }

    let pam = opts.pam || opts.tupltype.is_some() || fmt.has_alpha();
    if pam {
        let tupltype = opts.tupltype.as_deref().unwrap_or(standard_tupltype(fmt));
        if is_natural {
            let fast = match fmt {
                PbmPixelFormat::Gray8 => Some(encode_p7_gray8(plane, w, h, tupltype)),
                PbmPixelFormat::Gray16Le => Some(encode_p7_gray16(plane, w, h, tupltype)),
                PbmPixelFormat::Rgb24 => Some(encode_p7_rgb8(plane, w, h, tupltype)),
                PbmPixelFormat::Rgb48Le => Some(encode_p7_rgb16(plane, w, h, tupltype)),
                PbmPixelFormat::Rgba => Some(encode_p7_rgba8(plane, w, h, tupltype)),
                PbmPixelFormat::Bgra => Some(encode_p7_bgra8(plane, w, h, tupltype)),
                PbmPixelFormat::Rgba64Le => Some(encode_p7_rgba16(plane, w, h, tupltype)),
                PbmPixelFormat::Ya8 => Some(encode_p7_ya8(plane, w, h, tupltype)),
                PbmPixelFormat::Ya16Le => Some(encode_p7_ya16(plane, w, h, tupltype)),
                _ => None,
            };
            if let Some(out) = fast {
                return out;
            }
        }
        return encode_generic(plane, fmt, w, h, maxval, Flavour::Pam(tupltype));
    }

    if is_natural {
        return match fmt {
            PbmPixelFormat::MonoBlack => encode_p4(plane, w, h),
            PbmPixelFormat::Gray8 => encode_p5_gray8(plane, w, h),
            PbmPixelFormat::Gray16Le => encode_p5_gray16(plane, w, h),
            PbmPixelFormat::Rgb24 => encode_p6_rgb8(plane, w, h),
            _ => encode_p6_rgb16(plane, w, h),
        };
    }
    encode_generic(plane, fmt, w, h, maxval, Flavour::Raw)
}

/// Read the `channels` samples of pixel `x` in `row` as integers in
/// the layout's native range (`0..=in_max`), writing them to `out` in
/// R, G, B, A order (`Bgra` reordered; bilevel as the PAM
/// `BLACKANDWHITE` sense, 1 = white).
fn pixel_samples(fmt: PbmPixelFormat, row: &[u8], x: usize, out: &mut [u32; 4]) {
    match fmt {
        PbmPixelFormat::MonoBlack | PbmPixelFormat::MonoWhite => {
            let bit = (row[x / 8] >> (7 - (x % 8))) & 1;
            out[0] = u32::from(bit ^ 1);
        }
        PbmPixelFormat::Gray8 => out[0] = u32::from(row[x]),
        PbmPixelFormat::Ya8 => {
            out[0] = u32::from(row[x * 2]);
            out[1] = u32::from(row[x * 2 + 1]);
        }
        PbmPixelFormat::Rgb24 => {
            for c in 0..3 {
                out[c] = u32::from(row[x * 3 + c]);
            }
        }
        PbmPixelFormat::Rgba => {
            for c in 0..4 {
                out[c] = u32::from(row[x * 4 + c]);
            }
        }
        PbmPixelFormat::Bgra => {
            out[0] = u32::from(row[x * 4 + 2]);
            out[1] = u32::from(row[x * 4 + 1]);
            out[2] = u32::from(row[x * 4]);
            out[3] = u32::from(row[x * 4 + 3]);
        }
        PbmPixelFormat::Gray16Le
        | PbmPixelFormat::Ya16Le
        | PbmPixelFormat::Rgb48Le
        | PbmPixelFormat::Rgba64Le => {
            let ch = fmt.channels();
            for (c, o) in out.iter_mut().enumerate().take(ch) {
                let off = (x * ch + c) * 2;
                *o = u32::from(u16::from_le_bytes([row[off], row[off + 1]]));
            }
        }
        PbmPixelFormat::GrayF32Le | PbmPixelFormat::RgbF32Le => {}
    }
}

/// The generic sample-wise writer for every non-natural `MAXVAL` and
/// for PAM `BLACKANDWHITE`: each sample is rescaled by round-half-up
/// from the layout's native range to `0..=maxval` and written as a
/// decimal token (`Ascii`), one byte / two big-endian bytes (`Raw`,
/// `Pam`). Slower than the natural-maxval memcpy writers, which is fine
/// for an explicitly requested flavour.
fn encode_generic(
    plane: &PbmPlane,
    fmt: PbmPixelFormat,
    w: usize,
    h: usize,
    maxval: u32,
    flavour: Flavour<'_>,
) -> Result<Vec<u8>> {
    let channels = fmt.channels();
    let in_max = natural_maxval(fmt);
    let row_bytes = fmt
        .row_bytes(w as u32)
        .ok_or_else(|| Error::invalid("Netpbm encoder: row-size overflow"))?;
    let scale = |v: u32| -> u32 {
        if in_max == maxval {
            v
        } else {
            ((u64::from(v) * u64::from(maxval) + u64::from(in_max) / 2) / u64::from(in_max)) as u32
        }
    };
    let magic = match (flavour, channels) {
        (Flavour::Pam(_), _) => Magic::P7Pam,
        (Flavour::Ascii, 1) if fmt.is_bilevel() => Magic::P1AsciiBitmap,
        (Flavour::Ascii, 1) => Magic::P2AsciiGraymap,
        (Flavour::Ascii, _) => Magic::P3AsciiPixmap,
        (Flavour::Raw, 1) if fmt.is_bilevel() => Magic::P4BinaryBitmap,
        (Flavour::Raw, 1) => Magic::P5BinaryGraymap,
        (Flavour::Raw, _) => Magic::P6BinaryPixmap,
    };
    let mut out = match flavour {
        Flavour::Pam(t) => header_pam(w, h, channels as u32, maxval, t),
        _ => header_pnm(
            magic,
            w,
            h,
            if fmt.is_bilevel() { None } else { Some(maxval) },
        ),
    };
    let mut px = [0u32; 4];
    let wide = maxval > 255;
    match flavour {
        Flavour::Ascii => {
            let mut samples: Vec<u16> = Vec::with_capacity(w * h * channels);
            for y in 0..h {
                let row = &plane.data[y * plane.stride..y * plane.stride + row_bytes];
                for x in 0..w {
                    pixel_samples(fmt, row, x, &mut px);
                    for &s in px.iter().take(channels) {
                        // P1 is the wire sense (1 = black), the inverse of
                        // the PAM sense `pixel_samples` reports.
                        let v = if fmt.is_bilevel() { s ^ 1 } else { scale(s) };
                        samples.push(v as u16);
                    }
                }
            }
            out.extend(encode_ascii_body(&samples, (w * channels) as u32));
        }
        Flavour::Raw if fmt.is_bilevel() => {
            // Natural P4 — only reachable through the bilevel short
            // circuit (bilevel has no MAXVAL knob), kept for completeness.
            return encode_p4(plane, w, h);
        }
        Flavour::Raw | Flavour::Pam(_) => {
            for y in 0..h {
                let row = &plane.data[y * plane.stride..y * plane.stride + row_bytes];
                for x in 0..w {
                    pixel_samples(fmt, row, x, &mut px);
                    for &s in px.iter().take(channels) {
                        let v = if fmt.is_bilevel() { s } else { scale(s) };
                        if wide {
                            out.extend_from_slice(&(v as u16).to_be_bytes());
                        } else {
                            out.push(v as u8);
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Deprecated pre-contract entry points (one release)
// ---------------------------------------------------------------------------

/// Encode a [`PbmImage`] into the closest matching binary Netpbm
/// variant.
///
/// Deprecated: use [`crate::encode`] with `EncodeOptions::default()`.
#[deprecated(note = "use oxideav_pbm::encode (IMAGE_CRATE_API)")]
#[allow(deprecated)]
pub fn encode_pbm(image: &PbmImage) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("PBM encoder: empty plane"));
    }
    encode_pbm_plane(&image.planes[0], image.format, image.width, image.height)
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
///
/// Deprecated: the flavour is selected by [`EncodeOptions`] fields
/// (`ascii`, `pam`, `tupltype`, `maxval`, `pfm_*`).
#[deprecated(note = "use oxideav_pbm::EncodeOptions fields (IMAGE_CRATE_API)")]
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
    /// Force `P2` plain-ASCII graymap. Valid for `Gray8` (MAXVAL 255)
    /// and `Gray16Le` (MAXVAL 65535) — `pgm(5)` permits a MAXVAL up to
    /// 65535 for the plain (ASCII) form too, the body is just decimal
    /// integers.
    Pnm2,
    /// Force `P3` plain-ASCII pixmap. Valid for `Rgb24` (MAXVAL 255) and
    /// `Rgb48Le` (MAXVAL 65535) — `ppm(5)` permits a MAXVAL up to 65535
    /// for the plain (ASCII) form too.
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
    /// Force Portable FloatMap (`Pf` for `GrayF32Le`, `PF` for `RgbF32Le`).
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
///
/// Deprecated: use [`crate::encode`] with [`EncodeOptions`].
#[deprecated(note = "use oxideav_pbm::encode with EncodeOptions (IMAGE_CRATE_API)")]
#[allow(deprecated)]
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
        PbmEncodeFormat::Pnm1 => match image.format {
            PbmPixelFormat::MonoBlack => Ok(emit_ascii_pbm_header_and_body(plane, w, h)),
            PbmPixelFormat::MonoWhite => Ok(emit_ascii_pbm_header_and_body(
                &invert_bilevel(plane, w, h),
                w,
                h,
            )),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P1"
            ))),
        },
        PbmEncodeFormat::Pnm2 => match image.format {
            PbmPixelFormat::Gray8 => Ok(emit_ascii_pgm_8(plane, w, h)),
            PbmPixelFormat::Gray16Le => Ok(emit_ascii_pgm_16(plane, w, h)),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P2"
            ))),
        },
        PbmEncodeFormat::Pnm3 => match image.format {
            PbmPixelFormat::Rgb24 => Ok(emit_ascii_ppm_8(plane, w, h)),
            PbmPixelFormat::Rgb48Le => Ok(emit_ascii_ppm_16(plane, w, h)),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P3"
            ))),
        },
        PbmEncodeFormat::Pnm4 => match image.format {
            PbmPixelFormat::MonoBlack => encode_p4(plane, w, h),
            PbmPixelFormat::MonoWhite => encode_p4(&invert_bilevel(plane, w, h), w, h),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P4"
            ))),
        },
        PbmEncodeFormat::Pnm5 => match image.format {
            PbmPixelFormat::Gray8 => encode_p5_gray8(plane, w, h),
            PbmPixelFormat::Gray16Le => encode_p5_gray16(plane, w, h),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P5"
            ))),
        },
        PbmEncodeFormat::Pnm6 => match image.format {
            PbmPixelFormat::Rgb24 => encode_p6_rgb8(plane, w, h),
            PbmPixelFormat::Rgb48Le => encode_p6_rgb16(plane, w, h),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P6"
            ))),
        },
        PbmEncodeFormat::Pam7 => match image.format {
            PbmPixelFormat::Gray8 => encode_p7_gray8(plane, w, h, "GRAYSCALE"),
            PbmPixelFormat::Gray16Le => encode_p7_gray16(plane, w, h, "GRAYSCALE"),
            PbmPixelFormat::Rgb24 => encode_p7_rgb8(plane, w, h, "RGB"),
            PbmPixelFormat::Rgb48Le => encode_p7_rgb16(plane, w, h, "RGB"),
            PbmPixelFormat::Rgba => encode_p7_rgba8(plane, w, h, "RGB_ALPHA"),
            PbmPixelFormat::Bgra => encode_p7_bgra8(plane, w, h, "RGB_ALPHA"),
            PbmPixelFormat::Rgba64Le => encode_p7_rgba16(plane, w, h, "RGB_ALPHA"),
            PbmPixelFormat::Ya8 => encode_p7_ya8(plane, w, h, "GRAYSCALE_ALPHA"),
            PbmPixelFormat::Ya16Le => encode_p7_ya16(plane, w, h, "GRAYSCALE_ALPHA"),
            other => Err(Error::unsupported(format!(
                "PBM encoder: pixel format {other:?} cannot be emitted as P7"
            ))),
        },
        PbmEncodeFormat::Pfm => match image.format {
            PbmPixelFormat::GrayF32Le | PbmPixelFormat::RgbF32Le => crate::pfm::encode_pfm_plane(
                plane,
                image.format,
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
///
/// Deprecated: build a [`PbmImage`] with [`PbmImage::packed`] and use
/// [`crate::encode`].
#[deprecated(note = "use oxideav_pbm::encode (IMAGE_CRATE_API)")]
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
        PbmPixelFormat::MonoWhite => encode_p4(&invert_bilevel(plane, w, h), w, h),
        PbmPixelFormat::Gray8 => encode_p5_gray8(plane, w, h),
        PbmPixelFormat::Gray16Le => encode_p5_gray16(plane, w, h),
        PbmPixelFormat::Rgb24 => encode_p6_rgb8(plane, w, h),
        PbmPixelFormat::Rgb48Le => encode_p6_rgb16(plane, w, h),
        PbmPixelFormat::Rgba => encode_p7_rgba8(plane, w, h, "RGB_ALPHA"),
        PbmPixelFormat::Bgra => encode_p7_bgra8(plane, w, h, "RGB_ALPHA"),
        PbmPixelFormat::Rgba64Le => encode_p7_rgba16(plane, w, h, "RGB_ALPHA"),
        PbmPixelFormat::Ya8 => encode_p7_ya8(plane, w, h, "GRAYSCALE_ALPHA"),
        PbmPixelFormat::Ya16Le => encode_p7_ya16(plane, w, h, "GRAYSCALE_ALPHA"),
        // Float maps have no integer Netpbm form — emit Portable
        // FloatMap (`Pf` / `PF`). Default to little-endian (no byte swap
        // from the little-endian in-memory plane) with a unit scale.
        PbmPixelFormat::GrayF32Le | PbmPixelFormat::RgbF32Le => {
            crate::pfm::encode_pfm_plane(plane, format, width, height, true, 1.0)
        }
    }
}

/// ASCII variant: emit P1/P2/P3 from a [`PbmImage`]. Less efficient
/// (≥ 3× larger on disk) but the man pages still document the plain
/// forms and some tools require them.
///
/// Deprecated: use [`crate::encode`] with `EncodeOptions::new().with_ascii(true)`.
#[deprecated(note = "use oxideav_pbm::encode with EncodeOptions::with_ascii (IMAGE_CRATE_API)")]
#[allow(deprecated)]
pub fn encode_pbm_ascii(image: &PbmImage) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("PBM ASCII encoder: empty plane"));
    }
    encode_pbm_ascii_plane(&image.planes[0], image.format, image.width, image.height)
}

/// ASCII variant: emit P1/P2/P3 from a single plane.
///
/// Deprecated: use [`crate::encode`] with `EncodeOptions::new().with_ascii(true)`.
#[deprecated(note = "use oxideav_pbm::encode with EncodeOptions::with_ascii (IMAGE_CRATE_API)")]
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
        PbmPixelFormat::MonoWhite => Ok(emit_ascii_pbm_header_and_body(
            &invert_bilevel(plane, w, h),
            w,
            h,
        )),
        PbmPixelFormat::Gray8 => Ok(emit_ascii_pgm_8(plane, w, h)),
        PbmPixelFormat::Gray16Le => Ok(emit_ascii_pgm_16(plane, w, h)),
        PbmPixelFormat::Rgb24 => Ok(emit_ascii_ppm_8(plane, w, h)),
        PbmPixelFormat::Rgb48Le => Ok(emit_ascii_ppm_16(plane, w, h)),
        other => Err(Error::unsupported(format!(
            "PBM ASCII encoder: pixel format {other:?} not supported"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Binary writers — one per output magic
// ---------------------------------------------------------------------------

fn header_pnm(magic: Magic, w: usize, h: usize, maxval: Option<u32>) -> Vec<u8> {
    // Route the on-disk magic literal through `Magic::wire_bytes()` so
    // the encoder no longer carries a parallel `b'4' / b'5' / b'6'`
    // digit table that drifts away from the typed `Magic` variants over
    // time. `wire_bytes()` is a `&'static [u8]` accessor — no allocation,
    // same code shape as the previous two `push` calls.
    debug_assert!(
        magic.is_pnm() && magic != Magic::P7Pam,
        "header_pnm only emits P1..=P6 magics; P7 PAM and PFM use dedicated writers"
    );
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(magic.wire_bytes());
    out.push(b'\n');
    out.extend_from_slice(format!("{w} {h}\n").as_bytes());
    if let Some(mv) = maxval {
        out.extend_from_slice(format!("{mv}\n").as_bytes());
    }
    out
}

fn encode_p4(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    // The crate's `MonoBlack` plane convention (`1 = black`, MSB-first
    // packed, row stride `w.div_ceil(8)`) is byte-for-byte identical
    // to the P4 wire format, so the body is a per-row memcpy from the
    // plane to the output (with a trailing-bit mask on the last byte
    // of each row when `w % 8 != 0`). The pre-r229 path unpacked the
    // input into a `w * h`-byte intermediate (`Vec<u8>` allocation,
    // ~307 KiB at 640×480) and then re-packed it through the per-bit
    // OR loop in `encode_p4_body`, which forced a scalar bit loop on
    // both the unpack and repack passes. The new path:
    //
    //   1. Pre-resizes the output `Vec` to header + body in one go,
    //      so each row is written into a `&mut [u8]` slice (no
    //      `Vec::push`/`extend` calls that would inhibit SIMD).
    //   2. Calls `copy_p4_row_msb` per row, which lowers to a
    //      vectorised memcpy + a single-byte trailing-bit mask.
    //
    // Net effect at 640×480: one ~307 KiB allocation gone, the inner
    // bit loops gone, the body work is a straight memcpy lane.
    let row_bytes = w.div_ceil(8);
    let mut out = header_pnm(Magic::P4BinaryBitmap, w, h, None);
    let body_start = out.len();
    out.resize(body_start + row_bytes * h, 0);
    for y in 0..h {
        let src = &plane.data[y * plane.stride..y * plane.stride + row_bytes];
        let dst = &mut out[body_start + y * row_bytes..body_start + (y + 1) * row_bytes];
        copy_p4_row_msb(src, dst, w);
    }
    Ok(out)
}

fn encode_p5_gray8(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pnm(Magic::P5BinaryGraymap, w, h, Some(255));
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w]);
    }
    Ok(out)
}

fn encode_p5_gray16(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pnm(Magic::P5BinaryGraymap, w, h, Some(65535));
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
    let mut out = header_pnm(Magic::P6BinaryPixmap, w, h, Some(255));
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w * 3]);
    }
    Ok(out)
}

fn encode_p6_rgb16(plane: &PbmPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pnm(Magic::P6BinaryPixmap, w, h, Some(65535));
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
    out.extend_from_slice(Magic::P7Pam.wire_bytes());
    out.push(b'\n');
    out.extend_from_slice(format!("WIDTH {w}\n").as_bytes());
    out.extend_from_slice(format!("HEIGHT {h}\n").as_bytes());
    out.extend_from_slice(format!("DEPTH {depth}\n").as_bytes());
    out.extend_from_slice(format!("MAXVAL {maxval}\n").as_bytes());
    out.extend_from_slice(format!("TUPLTYPE {tupltype}\n").as_bytes());
    out.extend_from_slice(b"ENDHDR\n");
    out
}

fn encode_p7_gray8(plane: &PbmPlane, w: usize, h: usize, tupltype: &str) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 1, 255, tupltype);
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w]);
    }
    Ok(out)
}

fn encode_p7_gray16(plane: &PbmPlane, w: usize, h: usize, tupltype: &str) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 1, 65535, tupltype);
    // Identical body shape to P5 16-bit (PAM `GRAYSCALE` with depth 1 is
    // a single-sample row-major stream); funnel the LE→BE swap through
    // the row-level `swap_bytes_u16_row` helper so the inner loop walks
    // `chunks_exact(2)` over a pre-sized `&mut [u8]` destination and
    // lowers to a vectorised swap (`REV16.16B` on aarch64; `pshufb` /
    // `vpshufb` on x86). Closes the round-217 symmetry gap that left
    // this path on the per-sample `out.push(chunk[1]); out.push(chunk[0])`
    // pattern while the P5 / P6 / P7 RGB / RGBA 16-bit siblings all
    // moved to the helper.
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

fn encode_p7_rgb8(plane: &PbmPlane, w: usize, h: usize, tupltype: &str) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 3, 255, tupltype);
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w * 3]);
    }
    Ok(out)
}

fn encode_p7_rgb16(plane: &PbmPlane, w: usize, h: usize, tupltype: &str) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 3, 65535, tupltype);
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

fn encode_p7_rgba8(plane: &PbmPlane, w: usize, h: usize, tupltype: &str) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 4, 255, tupltype);
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w * 4]);
    }
    Ok(out)
}

fn encode_p7_bgra8(plane: &PbmPlane, w: usize, h: usize, tupltype: &str) -> Result<Vec<u8>> {
    // Reorder BGRA → RGBA on the way out so the file declares RGB_ALPHA
    // and any decoder reads them back as such. The per-row channel
    // shuffle is handled by `binary::bgra_to_rgba_row`, which walks
    // `chunks_exact(4)` zipped with `chunks_exact_mut(4)` over a
    // pre-resized `&mut [u8]` destination so LLVM can lower the inner
    // four-byte permutation to a vector lane shuffle (`TBL.16B` on
    // aarch64, `pshufb` / `vpshufb` on x86). Same shape as the
    // round-217 `swap_bytes_u16_row` and round-229 `copy_p4_row_msb`
    // helpers; closes the symmetry gap that left this path on the
    // per-pixel `out.push(px[2]); out.push(px[1]); …` pattern while
    // the other 8-bit binary encoders (P5 / P6 / P7 RGB / P7 RGBA /
    // P7 GRAYSCALE_ALPHA) all run `extend_from_slice` over a
    // contiguous row.
    let row_bytes = w * 4;
    let mut out = header_pam(w, h, 4, 255, tupltype);
    let body_start = out.len();
    out.resize(body_start + row_bytes * h, 0);
    for y in 0..h {
        let src = &plane.data[y * plane.stride..y * plane.stride + row_bytes];
        let dst = &mut out[body_start + y * row_bytes..body_start + (y + 1) * row_bytes];
        bgra_to_rgba_row(src, dst);
    }
    Ok(out)
}

fn encode_p7_rgba16(plane: &PbmPlane, w: usize, h: usize, tupltype: &str) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 4, 65535, tupltype);
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

fn encode_p7_ya8(plane: &PbmPlane, w: usize, h: usize, tupltype: &str) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 2, 255, tupltype);
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w * 2]);
    }
    Ok(out)
}

fn encode_p7_ya16(plane: &PbmPlane, w: usize, h: usize, tupltype: &str) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 2, 65535, tupltype);
    // Two 16-bit channels per pixel (Y, A) held little-endian in the
    // plane; the wire wants big-endian. The row-level swap is
    // channel-agnostic (it just walks 2-byte samples), so the same
    // helper the P5 / P6 / P7 RGB / RGBA 16-bit paths use applies
    // unchanged.
    let row_bytes = w * 4;
    let body_start = out.len();
    out.resize(body_start + row_bytes * h, 0);
    for y in 0..h {
        let src = &plane.data[y * plane.stride..y * plane.stride + row_bytes];
        let dst = &mut out[body_start + y * row_bytes..body_start + (y + 1) * row_bytes];
        swap_bytes_u16_row(src, dst);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// ASCII writers (P1/P2/P3)
// ---------------------------------------------------------------------------

fn emit_ascii_pbm_header_and_body(plane: &PbmPlane, w: usize, h: usize) -> Vec<u8> {
    // Direct bit-to-ASCII writer — avoids the temporary `Vec<u16>` the
    // generic `encode_ascii_body` would otherwise need.
    let mut out = header_pnm(Magic::P1AsciiBitmap, w, h, None);
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
    let mut out = header_pnm(Magic::P2AsciiGraymap, w, h, Some(255));
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
    let mut out = header_pnm(Magic::P3AsciiPixmap, w, h, Some(255));
    out.extend(encode_ascii_body_u8(&samples, w * 3));
    out
}

fn emit_ascii_pgm_16(plane: &PbmPlane, w: usize, h: usize) -> Vec<u8> {
    // P2 / Gray16Le: the plane holds little-endian u16 samples; ASCII
    // PGM samples are plain decimal integers, so read each LE pair into a
    // native `u16` and run the generic decimal writer. `pgm(5)` permits
    // a MAXVAL up to 65535 for the plain (ASCII) form, same as the raw
    // form — the only difference is the body encoding.
    let mut samples: Vec<u16> = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = &plane.data[y * plane.stride..y * plane.stride + w * 2];
        for px in row.chunks_exact(2) {
            samples.push(u16::from_le_bytes([px[0], px[1]]));
        }
    }
    let mut out = header_pnm(Magic::P2AsciiGraymap, w, h, Some(65535));
    out.extend(encode_ascii_body(&samples, w as u32));
    out
}

fn emit_ascii_ppm_16(plane: &PbmPlane, w: usize, h: usize) -> Vec<u8> {
    // P3 / Rgb48Le: three little-endian u16 channels per pixel. The
    // column stride for the readability line-break is `w * 3` samples
    // (three per pixel), matching the 8-bit P3 writer.
    let mut samples: Vec<u16> = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let row = &plane.data[y * plane.stride..y * plane.stride + w * 6];
        for px in row.chunks_exact(2) {
            samples.push(u16::from_le_bytes([px[0], px[1]]));
        }
    }
    let mut out = header_pnm(Magic::P3AsciiPixmap, w, h, Some(65535));
    out.extend(encode_ascii_body(&samples, (w * 3) as u32));
    out
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;

    fn make_image(
        format: PbmPixelFormat,
        w: u32,
        h: u32,
        stride: usize,
        data: Vec<u8>,
    ) -> PbmImage {
        PbmImage::packed(w, h, format, stride, data).unwrap()
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
    fn ascii_p2_gray16_emits_maxval_65535_and_round_trips() {
        // Gray16Le plane → P2 ASCII PGM with MAXVAL 65535.
        let img = make_image(
            PbmPixelFormat::Gray16Le,
            3,
            1,
            6,
            // LE samples: 0x0001, 0xFFFF, 0x1234
            vec![0x01, 0x00, 0xFF, 0xFF, 0x34, 0x12],
        );
        let bytes = encode_pbm_ascii(&img).unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("P2\n3 1\n65535\n"), "header: {s:?}");
        assert!(s.contains("1 65535 4660"), "body: {s:?}");
        // Round-trip: decode back and confirm the plane bytes match.
        let (back, fmt) = crate::decoder::decode_pbm(&bytes).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Gray16Le);
        assert_eq!(
            back.planes[0].data,
            vec![0x01, 0x00, 0xFF, 0xFF, 0x34, 0x12]
        );
    }

    #[test]
    fn ascii_p3_rgb48_emits_maxval_65535_and_round_trips() {
        // Rgb48Le plane → P3 ASCII PPM with MAXVAL 65535.
        let img = make_image(
            PbmPixelFormat::Rgb48Le,
            2,
            1,
            12,
            // pixel 0 = (0x0100, 0x0200, 0x0300), pixel 1 = (0xFFFF, 0, 0x8000)
            vec![
                0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x80,
            ],
        );
        let bytes = encode_pbm_ascii(&img).unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("P3\n2 1\n65535\n"), "header: {s:?}");
        // Six samples: 256 512 768 65535 0 32768
        assert!(s.contains("256 512 768 65535 0 32768"), "body: {s:?}");
        let (back, fmt) = crate::decoder::decode_pbm(&bytes).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Rgb48Le);
        assert_eq!(back.planes[0].data, img.planes[0].data);
    }

    #[test]
    fn explicit_pnm2_pnm3_accept_16bit_formats() {
        let g = make_image(PbmPixelFormat::Gray16Le, 1, 1, 2, vec![0x34, 0x12]);
        let gb = encode_pbm_with_format(&g, PbmEncodeFormat::Pnm2).unwrap();
        assert!(gb.starts_with(b"P2\n1 1\n65535\n"));
        let c = make_image(
            PbmPixelFormat::Rgb48Le,
            1,
            1,
            6,
            vec![0x34, 0x12, 0x78, 0x56, 0xBC, 0x9A],
        );
        let cb = encode_pbm_with_format(&c, PbmEncodeFormat::Pnm3).unwrap();
        assert!(cb.starts_with(b"P3\n1 1\n65535\n"));
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

    #[test]
    fn encode_p4_byte_aligned_width_round_trips() {
        // 16 px = exact 2 byte-row width — no trailing-bit mask
        // involvement. The body must match the input plane bytes.
        let img = make_image(
            PbmPixelFormat::MonoBlack,
            16,
            2,
            2,
            vec![0b1010_1100, 0b1111_0010, 0b0101_0101, 0b1110_0001],
        );
        let bytes = encode_pbm(&img).unwrap();
        assert!(bytes.starts_with(b"P4\n16 2\n"));
        let body = &bytes[bytes.len() - 4..];
        assert_eq!(body, &[0b1010_1100, 0b1111_0010, 0b0101_0101, 0b1110_0001]);
    }

    #[test]
    fn encode_p4_unaligned_width_zeros_trailing_pad() {
        // 11 px = 2 bytes per row with 5 padding bits at the tail of
        // each row. The encoder must zero those padding bits regardless
        // of what the source plane held there, so the on-disk bytes are
        // canonical and the round-205-style memcpy fast path doesn't
        // leak input garbage.
        let img = make_image(
            PbmPixelFormat::MonoBlack,
            11,
            1,
            2,
            // Dirty padding (bottom 5 bits set) on the input row.
            vec![0b1010_1100, 0b1111_1111],
        );
        let bytes = encode_pbm(&img).unwrap();
        let body = &bytes[bytes.len() - 2..];
        // Used bits in byte 1 are pixels 8/9/10 (= 1/1/1) so top 3
        // bits = 0b111; remaining 5 bits must be zero.
        assert_eq!(body, &[0b1010_1100, 0b1110_0000]);
    }

    #[test]
    fn encode_p4_strided_plane_matches_unstrided() {
        // The plane.stride may exceed row_bytes (e.g. a caller's image
        // buffer is padded for alignment). The encoder must walk
        // exactly `row_bytes` per row and ignore the trailing stride
        // padding. Build the same image twice — once tight, once
        // padded — and assert the two outputs match byte-for-byte.
        let tight = make_image(
            PbmPixelFormat::MonoBlack,
            11,
            3,
            2,
            vec![
                0b1010_1100,
                0b1110_0000, // row 0
                0b0101_0101,
                0b0100_0000, // row 1
                0b1111_0000,
                0b1000_0000, // row 2
            ],
        );
        let padded = make_image(
            PbmPixelFormat::MonoBlack,
            11,
            3,
            4, // stride = 4 bytes per row (2 used + 2 padding)
            vec![
                0b1010_1100,
                0b1110_0000,
                0xFF,
                0xFF, // row 0 + padding garbage
                0b0101_0101,
                0b0100_0000,
                0xCC,
                0xCC,
                0b1111_0000,
                0b1000_0000,
                0xAA,
                0xAA,
            ],
        );
        assert_eq!(encode_pbm(&tight).unwrap(), encode_pbm(&padded).unwrap());
    }

    #[test]
    fn explicit_format_pam7_gray16_be_swap() {
        // P7 GRAYSCALE 16-bit must emit the same big-endian byte sequence
        // as P5 16-bit for the same source LE plane. Regression for the
        // round-217 symmetry gap: `encode_p7_gray16` was the only 16-bit
        // encode path still using per-sample `out.push(chunk[1]);
        // out.push(chunk[0])`. After the round-222 refactor the helper
        // is shared, so the body bytes must agree with the canonical
        // P5 path.
        let img = make_image(
            PbmPixelFormat::Gray16Le,
            3,
            2,
            6,
            // Six LE samples covering high/low byte mixes: 0x1234,
            // 0x00FF, 0xFF00, 0xCAFE, 0xDEAD, 0xBEEF.
            vec![
                0x34, 0x12, 0xff, 0x00, 0x00, 0xff, 0xfe, 0xca, 0xad, 0xde, 0xef, 0xbe,
            ],
        );
        let pam = encode_pbm_with_format(&img, PbmEncodeFormat::Pam7).unwrap();
        let p5 = encode_pbm_with_format(&img, PbmEncodeFormat::Pnm5).unwrap();
        // PAM header is longer; compare the trailing 12 body bytes only.
        let pam_body = &pam[pam.len() - 12..];
        let p5_body = &p5[p5.len() - 12..];
        assert_eq!(pam_body, p5_body);
        // Spot-check: 0x1234 LE → 0x12 0x34 on disk.
        assert_eq!(
            pam_body,
            &[0x12, 0x34, 0x00, 0xff, 0xff, 0x00, 0xca, 0xfe, 0xde, 0xad, 0xbe, 0xef]
        );
        // The PAM header must declare DEPTH 1 + GRAYSCALE + MAXVAL 65535.
        let hdr_end = pam.iter().position(|&b| b == 0x12).unwrap();
        let s = std::str::from_utf8(&pam[..hdr_end]).unwrap();
        assert!(s.contains("DEPTH 1"));
        assert!(s.contains("TUPLTYPE GRAYSCALE"));
        assert!(s.contains("MAXVAL 65535"));
    }

    #[test]
    fn encode_p7_ya16_declares_grayscale_alpha_and_swaps_to_be() {
        // `Ya16Le` → P7 GRAYSCALE_ALPHA at maxval 65535: the header must
        // declare DEPTH 2 and the body must hold big-endian (Y, A)
        // sample pairs swapped from the plane's little-endian layout.
        let img = make_image(
            PbmPixelFormat::Ya16Le,
            2,
            1,
            8,
            // Two LE (Y, A) pixels: (0x1234, 0xABCD) + (0x00FF, 0xFF00).
            vec![0x34, 0x12, 0xCD, 0xAB, 0xFF, 0x00, 0x00, 0xFF],
        );
        let bytes = encode_pbm(&img).unwrap();
        assert!(bytes.starts_with(b"P7\n"));
        let body = &bytes[bytes.len() - 8..];
        assert_eq!(body, &[0x12, 0x34, 0xAB, 0xCD, 0x00, 0xFF, 0xFF, 0x00]);
        let s = std::str::from_utf8(&bytes[..bytes.len() - 8]).unwrap();
        assert!(s.contains("DEPTH 2"));
        assert!(s.contains("TUPLTYPE GRAYSCALE_ALPHA"));
        assert!(s.contains("MAXVAL 65535"));
        // The explicit `Pam7` selector must agree byte-for-byte with
        // the auto-binary route (P7 is the only home for Ya16Le).
        let explicit = encode_pbm_with_format(&img, PbmEncodeFormat::Pam7).unwrap();
        assert_eq!(bytes, explicit);
    }

    #[test]
    fn encode_p7_bgra_swaps_to_rgb_alpha_body() {
        // BGRA in / RGB_ALPHA out: the on-disk byte sequence must be
        // R/G/B/A per pixel even though the input plane is laid out
        // B/G/R/A. Regression for the round-253 `bgra_to_rgba_row`
        // refactor — the inner per-pixel channel shuffle moved from a
        // per-byte `Vec::push` loop to a row-level
        // `chunks_exact(4)` zip, so this guards against any
        // accidental index slip.
        let img = make_image(
            PbmPixelFormat::Bgra,
            2,
            1,
            8,
            // Two BGRA pixels: (B=0x10,G=0x20,R=0x30,A=0x40) +
            // (B=0xAB,G=0xCD,R=0xEF,A=0x12).
            vec![0x10, 0x20, 0x30, 0x40, 0xab, 0xcd, 0xef, 0x12],
        );
        let bytes = encode_pbm(&img).unwrap();
        let body = &bytes[bytes.len() - 8..];
        // R/G/B/A on disk: pixel 0 = (0x30, 0x20, 0x10, 0x40);
        // pixel 1 = (0xef, 0xcd, 0xab, 0x12).
        assert_eq!(body, &[0x30, 0x20, 0x10, 0x40, 0xef, 0xcd, 0xab, 0x12]);
        // The PAM header must declare DEPTH 4 + RGB_ALPHA + MAXVAL
        // 255, not the input's BGRA layout — the on-disk file is
        // never tagged BGRA.
        let hdr_end = bytes.len() - 8;
        let s = std::str::from_utf8(&bytes[..hdr_end]).unwrap();
        assert!(s.contains("DEPTH 4"));
        assert!(s.contains("TUPLTYPE RGB_ALPHA"));
        assert!(s.contains("MAXVAL 255"));
    }

    #[test]
    fn encode_p7_bgra_matches_canonical_rgba_after_swap() {
        // A BGRA plane and an Rgba plane that holds the same pixels
        // with channels pre-swapped must produce byte-for-byte
        // identical Netpbm output (same RGB_ALPHA header + same body).
        // Doubles as a regression that the helper does not also touch
        // the G or A channels.
        let bgra = make_image(
            PbmPixelFormat::Bgra,
            3,
            2,
            12,
            vec![
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
                0x0c, // row 0
                0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
                0x18, // row 1
            ],
        );
        let rgba = make_image(
            PbmPixelFormat::Rgba,
            3,
            2,
            12,
            // Same pixels with channels pre-swapped: (R, G, B, A)
            // per pixel where the BGRA source had (B, G, R, A).
            vec![
                0x03, 0x02, 0x01, 0x04, 0x07, 0x06, 0x05, 0x08, 0x0b, 0x0a, 0x09, 0x0c, 0x0f, 0x0e,
                0x0d, 0x10, 0x13, 0x12, 0x11, 0x14, 0x17, 0x16, 0x15, 0x18,
            ],
        );
        assert_eq!(encode_pbm(&bgra).unwrap(), encode_pbm(&rgba).unwrap());
    }

    #[test]
    fn encode_p7_bgra_strided_plane_matches_unstrided() {
        // `plane.stride` may exceed `width * 4` when the caller's
        // buffer has trailing row padding. The encoder must walk
        // exactly `width * 4` bytes per row and ignore the stride
        // padding. Mirrors `encode_p4_strided_plane_matches_unstrided`
        // for the BGRA path so the row-level helper plus the stride
        // arithmetic stay in step.
        let tight = make_image(
            PbmPixelFormat::Bgra,
            2,
            2,
            8,
            vec![
                0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, // row 0
                0x11, 0x21, 0x31, 0x41, 0x51, 0x61, 0x71, 0x81, // row 1
            ],
        );
        let padded = make_image(
            PbmPixelFormat::Bgra,
            2,
            2,
            12, // stride = 12 bytes per row (8 used + 4 padding)
            vec![
                0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0xff, 0xff, 0xff, 0xff, 0x11, 0x21,
                0x31, 0x41, 0x51, 0x61, 0x71, 0x81, 0xee, 0xee, 0xee, 0xee,
            ],
        );
        assert_eq!(encode_pbm(&tight).unwrap(), encode_pbm(&padded).unwrap());
    }
}
