//! Netpbm encoder.
//!
//! Picks an output magic from the input [`PixelFormat`]:
//!
//! | PixelFormat        | Output |
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

use oxideav_core::Encoder;
use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, Packet, PixelFormat, Result, TimeBase, VideoFrame,
};

use crate::ascii::encode_ascii_body;
use crate::binary::encode_p4_body;

pub fn make_encoder(params: &CodecParameters) -> Result<Box<dyn Encoder>> {
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

struct PbmEncoder {
    codec_id: CodecId,
    out_params: CodecParameters,
    pending: Option<Vec<u8>>,
    eof: bool,
}

impl Encoder for PbmEncoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn output_params(&self) -> &CodecParameters {
        &self.out_params
    }
    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        let vf = match frame {
            Frame::Video(v) => v,
            _ => return Err(Error::invalid("PBM encoder: expected video frame")),
        };
        let format = self.out_params.pixel_format.ok_or_else(|| {
            Error::invalid("PBM encoder: pixel_format missing in CodecParameters")
        })?;
        let width = self
            .out_params
            .width
            .ok_or_else(|| Error::invalid("PBM encoder: width missing in CodecParameters"))?;
        let height = self
            .out_params
            .height
            .ok_or_else(|| Error::invalid("PBM encoder: height missing in CodecParameters"))?;
        let bytes = encode_pbm(vf, format, width, height)?;
        self.pending = Some(bytes);
        Ok(())
    }
    fn receive_packet(&mut self) -> Result<Packet> {
        match self.pending.take() {
            Some(bytes) => {
                let mut pkt = Packet::new(0, TimeBase::new(1, 1), bytes);
                pkt.flags.keyframe = true;
                Ok(pkt)
            }
            None => {
                if self.eof {
                    Err(Error::Eof)
                } else {
                    Err(Error::NeedMore)
                }
            }
        }
    }
    fn flush(&mut self) -> Result<()> {
        self.eof = true;
        Ok(())
    }
}

/// Encode a [`VideoFrame`] into the closest matching binary Netpbm
/// variant.
pub fn encode_pbm(
    frame: &VideoFrame,
    format: PixelFormat,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    if frame.planes.is_empty() {
        return Err(Error::invalid("PBM encoder: empty plane"));
    }
    let plane = &frame.planes[0];
    if plane.data.len() < plane.stride * h {
        return Err(Error::invalid("PBM encoder: plane truncated"));
    }
    match format {
        PixelFormat::MonoBlack => encode_p4(plane, w, h),
        PixelFormat::Gray8 => encode_p5_gray8(plane, w, h),
        PixelFormat::Gray16Le => encode_p5_gray16(plane, w, h),
        PixelFormat::Rgb24 => encode_p6_rgb8(plane, w, h),
        PixelFormat::Rgb48Le => encode_p6_rgb16(plane, w, h),
        PixelFormat::Rgba => encode_p7_rgba8(plane, w, h),
        PixelFormat::Bgra => encode_p7_bgra8(plane, w, h),
        PixelFormat::Rgba64Le => encode_p7_rgba16(plane, w, h),
        PixelFormat::Ya8 => encode_p7_ya8(plane, w, h),
        other => Err(Error::unsupported(format!(
            "PBM encoder: pixel format {other:?} not supported in round 1"
        ))),
    }
}

/// ASCII variant: emit P1/P2/P3 by transcoding the binary output. Less
/// efficient (≥ 3× larger on disk) but the man pages still document
/// the plain forms and some tools require them.
pub fn encode_pbm_ascii(
    frame: &VideoFrame,
    format: PixelFormat,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    let plane = &frame.planes[0];
    match format {
        PixelFormat::MonoBlack => Ok(emit_ascii_pbm_header_and_body(plane, w, h)),
        PixelFormat::Gray8 => Ok(emit_ascii_pgm_8(plane, w, h)),
        PixelFormat::Rgb24 => Ok(emit_ascii_ppm_8(plane, w, h)),
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

fn encode_p4(plane: &oxideav_core::VideoPlane, w: usize, h: usize) -> Result<Vec<u8>> {
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

fn encode_p5_gray8(plane: &oxideav_core::VideoPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pnm(b'5', w, h, Some(255));
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w]);
    }
    Ok(out)
}

fn encode_p5_gray16(plane: &oxideav_core::VideoPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pnm(b'5', w, h, Some(65535));
    for y in 0..h {
        let row = &plane.data[y * plane.stride..y * plane.stride + w * 2];
        // `Gray16Le` stores LE; on-disk Netpbm wants BE.
        for chunk in row.chunks_exact(2) {
            out.push(chunk[1]);
            out.push(chunk[0]);
        }
    }
    Ok(out)
}

fn encode_p6_rgb8(plane: &oxideav_core::VideoPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pnm(b'6', w, h, Some(255));
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w * 3]);
    }
    Ok(out)
}

fn encode_p6_rgb16(plane: &oxideav_core::VideoPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pnm(b'6', w, h, Some(65535));
    for y in 0..h {
        let row = &plane.data[y * plane.stride..y * plane.stride + w * 6];
        for chunk in row.chunks_exact(2) {
            out.push(chunk[1]);
            out.push(chunk[0]);
        }
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

fn encode_p7_rgba8(plane: &oxideav_core::VideoPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 4, 255, "RGB_ALPHA");
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w * 4]);
    }
    Ok(out)
}

fn encode_p7_bgra8(plane: &oxideav_core::VideoPlane, w: usize, h: usize) -> Result<Vec<u8>> {
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

fn encode_p7_rgba16(plane: &oxideav_core::VideoPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 4, 65535, "RGB_ALPHA");
    for y in 0..h {
        let row = &plane.data[y * plane.stride..y * plane.stride + w * 8];
        // LE → BE per channel.
        for chunk in row.chunks_exact(2) {
            out.push(chunk[1]);
            out.push(chunk[0]);
        }
    }
    Ok(out)
}

fn encode_p7_ya8(plane: &oxideav_core::VideoPlane, w: usize, h: usize) -> Result<Vec<u8>> {
    let mut out = header_pam(w, h, 2, 255, "GRAYSCALE_ALPHA");
    for y in 0..h {
        out.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + w * 2]);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// ASCII writers (P1/P2/P3)
// ---------------------------------------------------------------------------

fn emit_ascii_pbm_header_and_body(plane: &oxideav_core::VideoPlane, w: usize, h: usize) -> Vec<u8> {
    let row_bytes = w.div_ceil(8);
    let mut samples: Vec<u16> = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = &plane.data[y * plane.stride..y * plane.stride + row_bytes];
        for x in 0..w {
            samples.push(((row[x / 8] >> (7 - (x % 8))) & 1) as u16);
        }
    }
    let mut out = header_pnm(b'1', w, h, None);
    out.extend(encode_ascii_body(&samples, w as u32));
    out
}

fn emit_ascii_pgm_8(plane: &oxideav_core::VideoPlane, w: usize, h: usize) -> Vec<u8> {
    let mut samples: Vec<u16> = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = &plane.data[y * plane.stride..y * plane.stride + w];
        for &b in row {
            samples.push(b as u16);
        }
    }
    let mut out = header_pnm(b'2', w, h, Some(255));
    out.extend(encode_ascii_body(&samples, w as u32));
    out
}

fn emit_ascii_ppm_8(plane: &oxideav_core::VideoPlane, w: usize, h: usize) -> Vec<u8> {
    let mut samples: Vec<u16> = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let row = &plane.data[y * plane.stride..y * plane.stride + w * 3];
        for &b in row {
            samples.push(b as u16);
        }
    }
    let mut out = header_pnm(b'3', w, h, Some(255));
    out.extend(encode_ascii_body(&samples, w as u32 * 3));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::{VideoFrame, VideoPlane};

    #[test]
    fn encode_p6_rgb8_smoke() {
        let plane = VideoPlane {
            stride: 6,
            data: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        };
        let vf = VideoFrame {
            pts: None,
            planes: vec![plane],
        };
        let bytes = encode_pbm(&vf, PixelFormat::Rgb24, 2, 2).unwrap();
        assert!(bytes.starts_with(b"P6\n2 2\n255\n"));
        let body = &bytes[bytes.iter().position(|&b| b == b'\n').unwrap() + 1..];
        let body = &body[body.iter().position(|&b| b == b'\n').unwrap() + 1..];
        let body = &body[body.iter().position(|&b| b == b'\n').unwrap() + 1..];
        assert_eq!(body, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }

    #[test]
    fn encode_p5_gray16_swaps_to_be() {
        let plane = VideoPlane {
            stride: 4,
            // LE input: 0x1234 then 0x5678
            data: vec![0x34, 0x12, 0x78, 0x56],
        };
        let vf = VideoFrame {
            pts: None,
            planes: vec![plane],
        };
        let bytes = encode_pbm(&vf, PixelFormat::Gray16Le, 2, 1).unwrap();
        assert!(bytes.starts_with(b"P5\n2 1\n65535\n"));
        // Last 4 bytes = BE samples
        assert_eq!(&bytes[bytes.len() - 4..], &[0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn encode_p7_rgba_emits_rgb_alpha_tupltype() {
        let plane = VideoPlane {
            stride: 4,
            data: vec![10, 20, 30, 40],
        };
        let vf = VideoFrame {
            pts: None,
            planes: vec![plane],
        };
        let bytes = encode_pbm(&vf, PixelFormat::Rgba, 1, 1).unwrap();
        assert!(bytes.starts_with(b"P7\n"));
        let s = std::str::from_utf8(&bytes[..bytes.len() - 4]).unwrap();
        assert!(s.contains("TUPLTYPE RGB_ALPHA"));
        assert_eq!(&bytes[bytes.len() - 4..], &[10, 20, 30, 40]);
    }
}
