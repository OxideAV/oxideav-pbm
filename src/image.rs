//! Standalone image container returned by `oxideav-pbm`'s framework-free
//! decode API and accepted by the standalone encode API.
//!
//! Defined here (rather than reusing `oxideav_core::VideoFrame`) so the
//! crate can be built with the default `registry` feature off — i.e.
//! without depending on `oxideav-core` at all. When the `registry`
//! feature is on the [`crate::registry`] module provides
//! `From<PbmImage> for oxideav_core::VideoFrame` (and the matching
//! [`PbmPixelFormat`] ↔ `oxideav_core::PixelFormat` mapping) so the
//! trait-side `Decoder` / `Encoder` impls keep working unchanged.

/// Pixel layout used by [`PbmImage`].
///
/// Each variant corresponds 1:1 to one of the `oxideav_core::PixelFormat`
/// variants the registry-side codec supports — the `registry` feature's
/// `From<PbmPixelFormat>` conversion just maps between the two. Callers
/// that build images programmatically pick the variant matching their
/// source data; the decoder picks the one matching the on-disk Netpbm
/// magic + bit depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PbmPixelFormat {
    /// 1 bit per pixel, MSB-first packed, 1 = black, rows padded to a
    /// byte. Matches PBM (P1 / P4) and PAM `BLACKANDWHITE`.
    MonoBlack,
    /// 8-bit single-channel grayscale, one byte per pixel.
    Gray8,
    /// 16-bit single-channel grayscale, little-endian, 2 bytes per pixel.
    Gray16Le,
    /// 8-bit packed RGB, 3 bytes per pixel.
    Rgb24,
    /// 16-bit packed RGB, little-endian, 6 bytes per pixel.
    Rgb48Le,
    /// 8-bit packed RGBA, 4 bytes per pixel.
    Rgba,
    /// 8-bit packed BGRA, 4 bytes per pixel (encode-side input only).
    Bgra,
    /// 16-bit packed RGBA, little-endian, 8 bytes per pixel.
    Rgba64Le,
    /// 8-bit grayscale + alpha (`Y, A`), 2 bytes per pixel.
    Ya8,
}

/// One image plane: row-major bytes plus the row stride in bytes.
///
/// Mirrors `oxideav_core::VideoPlane` so the registry-side conversion
/// is a trivial field-by-field copy.
#[derive(Debug, Clone)]
pub struct PbmPlane {
    /// Bytes per row in `data` (may exceed the logical row width when
    /// padding is required by the chosen pixel format).
    pub stride: usize,
    /// Raw plane bytes, packed `stride` × number of rows.
    pub data: Vec<u8>,
}

/// One decoded Netpbm frame, framework-free shape.
///
/// `pts` is `None` for the standalone [`crate::decode_pbm`] entry point
/// — that function operates on a single isolated file buffer without
/// packet timing information. The registry-backed `Decoder` impl still
/// passes `pts` through from the surrounding `Packet`.
#[derive(Debug, Clone)]
pub struct PbmImage {
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Pixel layout the planes carry.
    pub pixel_format: PbmPixelFormat,
    /// One [`PbmPlane`] per plane. Every Netpbm pixel format ships in
    /// a single packed plane today, so this is always `len() == 1`.
    pub planes: Vec<PbmPlane>,
    /// Optional presentation timestamp. Always `None` from the
    /// standalone decode path.
    pub pts: Option<i64>,
}
