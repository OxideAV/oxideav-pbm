//! Pure-Rust Netpbm (PBM/PGM/PPM/PNM/PAM) + Portable FloatMap image
//! codec + container.
//!
//! Covers all eight Netpbm magic numbers — the seven classic PNM
//! variants plus PAM (P7) — and the floating-point Portable FloatMap
//! sibling (`Pf` / `PF`) in one self-contained crate. Spec sources are
//! the staged family reference (`docs/image/pbm/`) and the Debevec PFM
//! reference (`docs/image/netpbm/pfm-portable-floatmap.md`). No
//! external implementation source was consulted.
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
//! # Standalone use (`IMAGE_CRATE_API`)
//!
//! The crate exposes the OxideAV image-crate contract at its root —
//! [`probe`], [`info`], [`decode`] / [`decode_with`] / [`decode_rgb8`] /
//! [`decode_rgba8`] / [`decode_all`] / [`decode_from`], [`encode`] /
//! [`encode_rgb8`] / [`encode_rgba8`] / [`encode_to`] — around
//! [`PbmImage`] (native layout, one packed [`Plane`], [`ColorInfo`],
//! [`Metadata`]), [`RgbImage`] / [`RgbaImage`], [`ImageInfo`], [`Frame`],
//! [`DecodeOptions`], [`EncodeOptions`], [`PixelFormat`] (=
//! [`PbmPixelFormat`], names mirroring `oxideav_core::PixelFormat`) and
//! [`Error`] (= [`PbmError`]). It builds with `default-features = false`
//! and no `oxideav-core`.
//!
//! ```
//! let bytes = oxideav_pbm::encode_rgb8(2, 1, &[255, 0, 0, 0, 0, 255], &Default::default())?;
//! assert!(oxideav_pbm::probe(&bytes));
//! let info = oxideav_pbm::info(&bytes)?;
//! assert_eq!((info.width, info.height, info.frames), (2, 1, 1));
//! let img = oxideav_pbm::decode(&bytes)?;             // PbmImage, native Rgb24
//! let rgba: Vec<u8> = img.to_rgba8();                  // 4 × width bytes per row
//! let opts = oxideav_pbm::EncodeOptions::default().with_ascii(true);
//! let p3 = oxideav_pbm::encode_rgba8(img.width(), img.height(), &rgba, &opts);
//! assert!(p3.is_err());                                // P1 / P2 / P3 cannot carry alpha
//! let p3 = oxideav_pbm::encode(&img, &opts)?;
//! assert!(p3.starts_with(b"P3\n"));
//! # Ok::<(), oxideav_pbm::Error>(())
//! ```
//!
//! Every PNM/PAM magic decodes to a [`PbmImage`] tagged with one of the
//! integer [`PbmPixelFormat`] variants; the two PFM magics decode to the
//! [`PbmPixelFormat::GrayF32Le`] / [`PbmPixelFormat::RgbF32Le`] native
//! float layouts (tone-scaled only inside [`PbmImage::to_rgb8`] /
//! [`PbmImage::to_rgba8`]). The encoder writes each layout in its
//! natural raw magic; [`EncodeOptions`] fields select the plain-text
//! forms, the PAM container, a custom `MAXVAL` / `TUPLTYPE`, and the
//! PFM byte order / scale. The depth APIs ([`parse_header`],
//! [`iter_pnm_header_comments`], the [`pfm`] scale helpers) keep their
//! names.
//!
//! Comments (`# … LF`) are tolerated everywhere the Netpbm grammar
//! permits them — in headers and in the bodies of P1/P2/P3 — and any
//! ASCII whitespace separates header tokens / ASCII samples.
//!
//! # Framework use
//!
//! The crate's default `registry` Cargo feature pulls in `oxideav-core`
//! and exposes `register` (`RuntimeContext`), `register_codecs` /
//! `register_containers`, the `make_decoder` / `make_encoder`
//! factories, and the frame bridge (`From<PbmImage> for VideoFrame`,
//! `PbmImage::from_video_frame`). The trait-side `Decoder` /
//! `Encoder` are thin adapters over the standalone functions.

pub mod api;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod ascii;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod binary;
#[cfg(feature = "registry")]
pub mod container;
pub mod decoder;
pub mod encoder;
pub mod error;
pub mod header;
pub mod image;
pub mod options;
pub mod pfm;
#[cfg(feature = "registry")]
pub mod registry;

/// Codec id for Netpbm image frames. All nine magics share this id —
/// the body itself is self-describing.
pub const CODEC_ID_STR: &str = "pbm";

// ---- the contract surface (IMAGE_CRATE_API) ---------------------------------
pub use api::{
    decode, decode_all, decode_all_with, decode_from, decode_rgb8, decode_rgba8, decode_with,
    encode, encode_all, encode_rgb8, encode_rgba8, encode_to, info, probe,
};
pub use error::{Error, PbmError, Result};
pub use image::{
    ColorInfo, ColorRange, Frame, ImageInfo, Metadata, PbmImage, PbmPixelFormat, PixelFormat,
    Plane, RgbImage, RgbaImage,
};
pub use options::{DecodeOptions, EncodeOptions};

// ---- depth APIs (keep their names) -------------------------------------------
pub use header::{
    iter_pnm_header_comments, parse_header, peek_magic, Header, Magic, PfmInfo, PnmHeaderComments,
    Tupltype,
};
pub use pfm::{
    apply_inverse_pfm_scale, apply_pfm_scale, decode_pfm, decode_pfm_consumed, decode_pfm_multi,
    decode_pfm_scaled, encode_pfm, encode_pfm_plane, encode_pfm_scaled, PfmHeaderInfo,
};

// ---- deprecated pre-contract entry points (one release) ----------------------
#[allow(deprecated)]
pub use decoder::{
    decode_pbm, decode_pbm_consumed, decode_pbm_header_consumed, decode_pbm_multi,
    decode_pbm_multi_with_headers,
};
#[allow(deprecated)]
pub use encoder::{
    encode_pbm, encode_pbm_ascii, encode_pbm_ascii_plane, encode_pbm_plane, encode_pbm_with_format,
    PbmEncodeFormat,
};
#[allow(deprecated)]
pub use image::PbmPlane;

/// Former name of [`probe`].
#[deprecated(note = "use oxideav_pbm::probe (IMAGE_CRATE_API)")]
pub fn probe_is_netpbm(input: &[u8]) -> bool {
    probe(input)
}

#[cfg(feature = "registry")]
pub use decoder::make_decoder;
#[cfg(feature = "registry")]
pub use encoder::make_encoder;
#[cfg(feature = "registry")]
pub use registry::{
    from_core_pixel_format, register, register_codecs, register_containers, to_core_pixel_format,
};
#[cfg(feature = "registry")]
#[allow(deprecated)]
pub use registry::{pbm_to_pixel_format, pixel_format_to_pbm};

#[cfg(feature = "registry")]
#[doc(hidden)]
pub use registry::__oxideav_entry;

#[cfg(test)]
#[allow(deprecated)]
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
        PbmImage::packed(w, h, PbmPixelFormat::Rgb24, w as usize * 3, data).unwrap()
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
        let src = PbmImage::packed(6, 4, PbmPixelFormat::Rgba, 24, data).unwrap();
        let bytes = encode_pbm(&src).unwrap();
        assert!(bytes.starts_with(b"P7\n"));
        let (back, fmt) = decode_pbm(&bytes).unwrap();
        assert_eq!(fmt, PbmPixelFormat::Rgba);
        assert_eq!(back.planes[0].data, src.planes[0].data);
    }
}
