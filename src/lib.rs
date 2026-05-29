//! Pure-Rust Netpbm (PBM/PGM/PPM/PNM/PAM) + Portable FloatMap image
//! codec + container.
//!
//! Covers all eight Netpbm magic numbers — the seven classic PNM
//! variants plus PAM (P7) — and the floating-point Portable FloatMap
//! sibling (`Pf` / `PF`) in one self-contained crate. Spec sources are
//! the Netpbm man pages (`pbm(5)`, `pgm(5)`, `ppm(5)`, `pnm(5)`,
//! `pam(5)`) and the Debevec PFM reference
//! (`docs/image/netpbm/pfm-portable-floatmap.md`). No external
//! implementation source was consulted.
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
//! | `Pf`  | PFM  | Binary   | 1 (gray)   | 32 (IEEE-754 float) |
//! | `PF`  | PFM  | Binary   | 3 (RGB)    | 32 (IEEE-754 float) |
//!
//! Every PNM/PAM magic decodes to a [`PbmImage`] tagged with one of the
//! integer [`PbmPixelFormat`] variants; the encoder picks the closest
//! binary form (P4/P5/P6/P7) for any supported pixel format. ASCII-form
//! output is also available via [`encode_pbm_ascii`]. The two PFM magics
//! decode/encode IEEE-754 binary32 samples (the
//! [`PbmPixelFormat::GrayF32`] / [`PbmPixelFormat::RgbF32`] variants) via
//! the dedicated [`decode_pfm`] / [`encode_pfm`] entry points (see the
//! [`pfm`] module); [`decode_pbm`] / [`encode_pbm`] also route `Pf` / `PF`
//! to that path automatically.
//!
//! Comments (`# … LF`) are tolerated everywhere the Netpbm spec
//! permits them — in headers and in the bodies of P1/P2/P3 — and any
//! ASCII whitespace separates header tokens / ASCII samples.
//!
//! ## Standalone vs registry-integrated
//!
//! The crate's default `registry` Cargo feature pulls in `oxideav-core`
//! and exposes the framework `Decoder` / `Encoder` trait surface plus
//! a [`registry::register`] entry point. Disable the feature
//! (`default-features = false`) for an `oxideav-core`-free build that
//! still exposes the standalone [`decode_pbm`] / [`encode_pbm`] /
//! [`encode_pbm_ascii`] API.

pub mod ascii;
pub mod binary;
#[cfg(feature = "registry")]
pub mod container;
pub mod decoder;
pub mod encoder;
pub mod error;
pub mod header;
pub mod image;
pub mod pfm;
#[cfg(feature = "registry")]
pub mod registry;

/// Codec id for Netpbm image frames. All eight magics share this id —
/// the body itself is self-describing.
pub const CODEC_ID_STR: &str = "pbm";

pub use decoder::decode_pbm;
pub use encoder::{
    encode_pbm, encode_pbm_ascii, encode_pbm_ascii_plane, encode_pbm_plane, encode_pbm_with_format,
    PbmEncodeFormat,
};
pub use error::{PbmError, Result};
pub use header::{parse_header, Header, Magic, PfmInfo, Tupltype};
pub use image::{PbmImage, PbmPixelFormat, PbmPlane};
pub use pfm::{decode_pfm, encode_pfm, encode_pfm_plane, PfmHeaderInfo};

#[cfg(feature = "registry")]
pub use registry::{
    __oxideav_entry, pbm_to_pixel_format, pixel_format_to_pbm, register, register_codecs,
    register_containers,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb_checker(w: u32, h: u32) -> PbmImage {
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let q = ((x & 1) + 2 * (y & 1)) as usize;
                let rgb = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]][q];
                data.extend_from_slice(&rgb);
            }
        }
        PbmImage {
            width: w,
            height: h,
            pixel_format: PbmPixelFormat::Rgb24,
            planes: vec![PbmPlane {
                stride: w as usize * 3,
                data,
            }],
            pts: None,
        }
    }

    #[test]
    fn p6_round_trip_pixel_exact() {
        let src = rgb_checker(16, 12);
        let bytes = encode_pbm(&src).unwrap();
        assert!(bytes.starts_with(b"P6\n"));
        let (back, fmt) = decode_pbm(&bytes).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Rgb24);
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
        let src = PbmImage {
            width: 6,
            height: 4,
            pixel_format: PbmPixelFormat::Rgba,
            planes: vec![PbmPlane { stride: 24, data }],
            pts: None,
        };
        let bytes = encode_pbm(&src).unwrap();
        assert!(bytes.starts_with(b"P7\n"));
        let (back, fmt) = decode_pbm(&bytes).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Rgba);
        assert_eq!(back.planes[0].data, src.planes[0].data);
    }
}
