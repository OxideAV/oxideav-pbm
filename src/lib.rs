//! Pure-Rust Netpbm (PBM/PGM/PPM/PNM/PAM) image codec + container.
//!
//! Covers all eight Netpbm magic numbers — the seven classic PNM
//! variants plus PAM (P7) — in one self-contained crate. Spec sources
//! are the Netpbm man pages: `pbm(5)`, `pgm(5)`, `ppm(5)`, `pnm(5)`,
//! `pam(5)`. No external implementation source was consulted.
//!
//! | Magic | Name | Encoding | Channels | Bit depth |
//! |-------|------|----------|----------|-----------|
//! | P1    | PBM  | ASCII    | 1 (binary) | 1 |
//! | P2    | PGM  | ASCII    | 1          | 8 or 16 |
//! | P3    | PPM  | ASCII    | 3 (RGB)    | 8 or 16 |
//! | P4    | PBM  | Binary   | 1 (binary) | 1 |
//! | P5    | PGM  | Binary   | 1          | 8 or 16 |
//! | P6    | PPM  | Binary   | 3 (RGB)    | 8 or 16 |
//! | P7    | PAM  | Binary   | 1-4 (depth + tupltype) | 1-16 (arbitrary `MAXVAL`) |
//!
//! All seven magics decode to an `oxideav-core` [`VideoFrame`]; the
//! encoder picks the closest binary form (P4/P5/P6/P7) for any
//! supported [`PixelFormat`]. ASCII-form output is also available via
//! [`encoder::encode_pbm_ascii`].
//!
//! Comments (`# … LF`) are tolerated everywhere the Netpbm spec
//! permits them — in headers and in the bodies of P1/P2/P3 — and any
//! ASCII whitespace separates header tokens / ASCII samples.
//!
//! The crate registers itself both as a codec (`pbm` codec id) and as
//! a container (extensions `.pbm`, `.pgm`, `.ppm`, `.pnm`, `.pam`)
//! since each Netpbm file is fully self-contained.

pub mod ascii;
pub mod binary;
pub mod container;
pub mod decoder;
pub mod encoder;
pub mod header;

use oxideav_core::ContainerRegistry;
use oxideav_core::{CodecCapabilities, CodecId, PixelFormat};
use oxideav_core::{CodecInfo, CodecRegistry};

/// Codec id for Netpbm image frames. All eight magics share this id —
/// the body itself is self-describing.
pub const CODEC_ID_STR: &str = "pbm";

pub fn register_codecs(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::video("pbm_sw")
        .with_intra_only(true)
        .with_lossless(true)
        .with_max_size(65535, 65535)
        .with_pixel_formats(vec![
            PixelFormat::MonoBlack,
            PixelFormat::Gray8,
            PixelFormat::Gray16Le,
            PixelFormat::Rgb24,
            PixelFormat::Rgb48Le,
            PixelFormat::Rgba,
            PixelFormat::Rgba64Le,
            PixelFormat::Ya8,
        ]);
    reg.register(
        CodecInfo::new(CodecId::new(CODEC_ID_STR))
            .capabilities(caps)
            .decoder(decoder::make_decoder)
            .encoder(encoder::make_encoder),
    );
}

pub fn register_containers(reg: &mut ContainerRegistry) {
    container::register(reg);
}

pub fn register(codecs: &mut CodecRegistry, containers: &mut ContainerRegistry) {
    register_codecs(codecs);
    register_containers(containers);
}

pub use decoder::decode_pbm;
pub use encoder::{encode_pbm, encode_pbm_ascii};
pub use header::{parse_header, Header, Magic, Tupltype};

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::{PixelFormat, VideoFrame, VideoPlane};

    fn rgb_checker(w: u32, h: u32) -> VideoFrame {
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let q = ((x & 1) + 2 * (y & 1)) as usize;
                let rgb = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]][q];
                data.extend_from_slice(&rgb);
            }
        }
        VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: w as usize * 3,
                data,
            }],
        }
    }

    #[test]
    fn p6_round_trip_pixel_exact() {
        let src = rgb_checker(16, 12);
        let bytes = encode_pbm(&src, PixelFormat::Rgb24, 16, 12).unwrap();
        assert!(bytes.starts_with(b"P6\n"));
        let (back, fmt) = decode_pbm(&bytes).unwrap();
        assert_eq!(fmt, PixelFormat::Rgb24);
        assert_eq!(back.planes[0].data, src.planes[0].data);
    }

    #[test]
    fn p7_round_trip_with_alpha() {
        let mut data = Vec::new();
        for y in 0..4 {
            for x in 0..6 {
                data.extend_from_slice(&[
                    (x as u8) * 40,
                    (y as u8) * 60,
                    255 - (x as u8) * 40,
                    if (x + y) & 1 == 0 { 255 } else { 64 },
                ]);
            }
        }
        let src = VideoFrame {
            pts: None,
            planes: vec![VideoPlane { stride: 24, data }],
        };
        let bytes = encode_pbm(&src, PixelFormat::Rgba, 6, 4).unwrap();
        assert!(bytes.starts_with(b"P7\n"));
        let (back, fmt) = decode_pbm(&bytes).unwrap();
        assert_eq!(fmt, PixelFormat::Rgba);
        assert_eq!(back.planes[0].data, src.planes[0].data);
    }
}
