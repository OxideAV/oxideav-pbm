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
//! | P7 RGB_ALPHA 8          | `Rgba`         |
//! | P7 RGB_ALPHA 16         | `Rgba64Le`     |
//!
//! 16-bit grayscale-with-alpha and BLACKANDWHITE_ALPHA fall back to a
//! 4-byte-per-pixel `Rgba` representation since the workspace's pixel
//! catalogue doesn't carry a `Ya16` variant — the alpha channel is
//! preserved either way.
//!
//! With the default `registry` feature on, the gated `PbmDecoder` trait
//! impl wraps [`decode_pbm`] for the `oxideav_core::Decoder` surface.

use crate::error::{PbmError as Error, Result};

use crate::ascii::decode_ascii;
use crate::binary::{decode_binary, DecodedSamples};
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
pub fn decode_pbm(input: &[u8]) -> Result<(PbmImage, PbmPixelFormat)> {
    let header = parse_header(input)?;
    let body = &input[header.data_offset..];
    let samples = if header.magic.is_ascii() {
        decode_ascii(&header, body)?
    } else {
        decode_binary(&header, body)?
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
    ))
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
    if let Some(t) = h.tupltype {
        match t {
            Tupltype::BlackAndWhiteAlpha | Tupltype::GrayscaleAlpha => RgbaLayout::GrayAlpha,
            Tupltype::RgbAlpha => RgbaLayout::Rgba,
            Tupltype::Rgb => RgbaLayout::RgbOpaque,
            _ => RgbaLayout::GrayOpaque,
        }
    } else {
        match depth {
            1 => RgbaLayout::GrayOpaque,
            2 => RgbaLayout::GrayAlpha,
            3 => RgbaLayout::RgbOpaque,
            4 => RgbaLayout::Rgba,
            _ => RgbaLayout::GrayOpaque,
        }
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
        Magic::P7Pam => match (h.tupltype, h.depth, h.maxval > 255) {
            (Some(Tupltype::BlackAndWhite), _, _) => PbmPixelFormat::MonoBlack,
            (Some(Tupltype::Grayscale), _, false) => PbmPixelFormat::Gray8,
            (Some(Tupltype::Grayscale), _, true) => PbmPixelFormat::Gray16Le,
            (Some(Tupltype::Rgb), _, false) => PbmPixelFormat::Rgb24,
            (Some(Tupltype::Rgb), _, true) => PbmPixelFormat::Rgb48Le,
            (Some(Tupltype::GrayscaleAlpha), _, false) => PbmPixelFormat::Ya8,
            (Some(Tupltype::GrayscaleAlpha), _, true) => PbmPixelFormat::Rgba, // no Ya16 in core
            (Some(Tupltype::BlackAndWhiteAlpha), _, _) => PbmPixelFormat::Rgba,
            (Some(Tupltype::RgbAlpha), _, false) => PbmPixelFormat::Rgba,
            (Some(Tupltype::RgbAlpha), _, true) => PbmPixelFormat::Rgba64Le,
            // Tuple type omitted — fall back on depth.
            (None, 1, false) => PbmPixelFormat::Gray8,
            (None, 1, true) => PbmPixelFormat::Gray16Le,
            (None, 2, false) => PbmPixelFormat::Ya8,
            (None, 2, true) => PbmPixelFormat::Rgba,
            (None, 3, false) => PbmPixelFormat::Rgb24,
            (None, 3, true) => PbmPixelFormat::Rgb48Le,
            (None, 4, false) => PbmPixelFormat::Rgba,
            (None, 4, true) => PbmPixelFormat::Rgba64Le,
            (_, d, _) => {
                return Err(Error::unsupported(format!(
                    "PAM: depth {d} with no recognised TUPLTYPE"
                )))
            }
        },
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
}
