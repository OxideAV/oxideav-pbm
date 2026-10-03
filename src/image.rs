//! The standalone image types: the shapes every `oxideav-<format>`
//! image crate shares (`IMAGE_CRATE_API`), specialised for the Netpbm
//! family.
//!
//! * [`PbmImage`] — the native-layout image [`crate::decode`] returns
//!   and [`crate::encode`] consumes: dimensions, a [`PixelFormat`] tag,
//!   one packed [`Plane`], [`ColorInfo`] and [`Metadata`]. Netpbm has
//!   no palette mechanism, so there is no `palette` field.
//! * [`RgbImage`] / [`RgbaImage`] — the tightly packed 8-bit raw paths
//!   ([`crate::decode_rgb8`] / [`crate::decode_rgba8`],
//!   [`PbmImage::to_rgb8`] / [`PbmImage::to_rgba8`]).
//! * [`Frame`] — one entry of [`crate::decode_all`] (a concatenated
//!   multi-image file).
//! * [`ImageInfo`] — what [`crate::info`] reads from the header.
//!
//! Defined here (rather than reusing `oxideav_core::VideoFrame`) so the
//! crate can be built with the default `registry` feature off — i.e.
//! without depending on `oxideav-core` at all. When the `registry`
//! feature is on, `crate::registry` provides `From<PbmImage> for
//! oxideav_core::VideoFrame` (and the matching [`PbmPixelFormat`] ↔
//! `oxideav_core::PixelFormat` mapping) so the trait-side `Decoder` /
//! `Encoder` impls are thin adapters over the same functions.

use std::time::Duration;

use crate::error::{PbmError, Result};
use crate::header::Header;

/// Pixel layout used by [`PbmImage`].
///
/// Variant names mirror `oxideav_core::PixelFormat` exactly, so the
/// `registry` feature's conversion is a 1:1 name match. Callers that
/// build images programmatically pick the variant matching their
/// source data; the decoder picks the one matching the on-disk Netpbm
/// magic + bit depth (see the table on [`crate::decoder`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PbmPixelFormat {
    /// 1 bit per pixel, MSB-first packed, 1 = black, rows padded to a
    /// byte. Matches PBM (P1 / P4) and PAM `BLACKANDWHITE` (whose bit
    /// sense the decoder normalises). The decoder's bilevel layout.
    MonoBlack,
    /// 1 bit per pixel, MSB-first packed, 1 = white, rows padded to a
    /// byte. Encode-side input only: the encoder inverts the bits into
    /// the P1 / P4 wire sense (1 = black); the decoder never produces it.
    MonoWhite,
    /// 8-bit single-channel grayscale, one byte per pixel.
    Gray8,
    /// 16-bit single-channel grayscale, little-endian, 2 bytes per pixel.
    Gray16Le,
    /// 8-bit grayscale + alpha (`Y, A`), 2 bytes per pixel.
    Ya8,
    /// 16-bit grayscale + alpha (`Y, A`), little-endian, 4 bytes per
    /// pixel. Decoded from / encoded to PAM `GRAYSCALE_ALPHA` at
    /// `MAXVAL` > 255.
    Ya16Le,
    /// 8-bit packed RGB, 3 bytes per pixel.
    Rgb24,
    /// 16-bit packed RGB, little-endian, 6 bytes per pixel.
    Rgb48Le,
    /// 8-bit packed RGBA, 4 bytes per pixel.
    Rgba,
    /// 8-bit packed BGRA, 4 bytes per pixel (encode-side input only;
    /// the encoder reorders to PAM `RGB_ALPHA`).
    Bgra,
    /// 16-bit packed RGBA, little-endian, 8 bytes per pixel.
    Rgba64Le,
    /// Single-channel IEEE-754 binary32 (32-bit float) grayscale, one
    /// 4-byte little-endian sample per pixel. High-dynamic-range linear
    /// light; the plane stores each float little-endian regardless of
    /// the on-disk byte order it was read from. Decoded from / encoded
    /// to the `Pf` Portable FloatMap form (see [`crate::pfm`]).
    GrayF32Le,
    /// 3-channel (R, G, B interleaved) IEEE-754 binary32 (32-bit float)
    /// colour, 12 bytes per pixel, each float little-endian. Decoded
    /// from / encoded to the `PF` Portable FloatMap form.
    RgbF32Le,
}

/// The contract name for [`PbmPixelFormat`].
pub type PixelFormat = PbmPixelFormat;

impl PbmPixelFormat {
    /// Former name of [`PbmPixelFormat::GrayF32Le`].
    #[allow(non_upper_case_globals)]
    #[deprecated(note = "renamed to PbmPixelFormat::GrayF32Le (mirrors oxideav_core::PixelFormat)")]
    pub const GrayF32: Self = Self::GrayF32Le;
    /// Former name of [`PbmPixelFormat::RgbF32Le`].
    #[allow(non_upper_case_globals)]
    #[deprecated(note = "renamed to PbmPixelFormat::RgbF32Le (mirrors oxideav_core::PixelFormat)")]
    pub const RgbF32: Self = Self::RgbF32Le;

    /// Every layout, in declaration order.
    pub const ALL: [Self; 13] = [
        Self::MonoBlack,
        Self::MonoWhite,
        Self::Gray8,
        Self::Gray16Le,
        Self::Ya8,
        Self::Ya16Le,
        Self::Rgb24,
        Self::Rgb48Le,
        Self::Rgba,
        Self::Bgra,
        Self::Rgba64Le,
        Self::GrayF32Le,
        Self::RgbF32Le,
    ];

    /// Number of colour/alpha channels per pixel.
    ///
    /// * `MonoBlack` / `MonoWhite` — 1 (a single packed bit plane).
    /// * `Gray8` / `Gray16Le` / `GrayF32Le` — 1.
    /// * `Ya8` / `Ya16Le` — 2 (luma + alpha).
    /// * `Rgb24` / `Rgb48Le` / `RgbF32Le` — 3.
    /// * `Rgba` / `Bgra` / `Rgba64Le` — 4.
    ///
    /// This is the logical channel count the on-disk Netpbm `DEPTH`
    /// (for PAM) or magic (for P1-P6 / `Pf` / `PF`) carries — it does
    /// not depend on the in-memory byte width of each channel.
    pub fn channels(self) -> usize {
        match self {
            Self::MonoBlack | Self::MonoWhite | Self::Gray8 | Self::Gray16Le | Self::GrayF32Le => 1,
            Self::Ya8 | Self::Ya16Le => 2,
            Self::Rgb24 | Self::Rgb48Le | Self::RgbF32Le => 3,
            Self::Rgba | Self::Bgra | Self::Rgba64Le => 4,
        }
    }

    /// Bits per channel held **in memory** for one decoded pixel.
    ///
    /// * `MonoBlack` / `MonoWhite` — 1 (one packed bit per pixel).
    /// * the `*8` / `Rgb24` / `Rgba` / `Bgra` integer formats — 8.
    /// * the `*16Le` integer formats — 16.
    /// * the `*F32Le` float formats — 32.
    pub fn bits_per_channel(self) -> usize {
        match self {
            Self::MonoBlack | Self::MonoWhite => 1,
            Self::Gray8 | Self::Rgb24 | Self::Rgba | Self::Bgra | Self::Ya8 => 8,
            Self::Gray16Le | Self::Rgb48Le | Self::Rgba64Le | Self::Ya16Le => 16,
            Self::GrayF32Le | Self::RgbF32Le => 32,
        }
    }

    /// Bytes occupied by one pixel in the in-memory plane.
    ///
    /// Returns `None` for the bilevel layouts ([`PbmPixelFormat::MonoBlack`]
    /// / [`PbmPixelFormat::MonoWhite`]), whose pixels are sub-byte (1 bit
    /// each, MSB-first packed with rows padded to a byte boundary) — the
    /// byte-per-row stride is `width.div_ceil(8)` rather than
    /// `width * bytes_per_pixel`. Every other format packs a whole
    /// number of bytes per pixel (`channels * bits_per_channel / 8`).
    pub fn bytes_per_pixel(self) -> Option<usize> {
        if self.is_bilevel() {
            return None;
        }
        Some(self.channels() * (self.bits_per_channel() / 8))
    }

    /// Minimum bytes one row of `width` pixels occupies in this layout
    /// (`width.div_ceil(8)` for the bilevel layouts, `width ×
    /// bytes_per_pixel` otherwise), or `None` on `usize` overflow.
    pub fn row_bytes(self, width: u32) -> Option<usize> {
        let w = width as usize;
        match self.bytes_per_pixel() {
            Some(bpp) => w.checked_mul(bpp),
            None => Some(w.div_ceil(8)),
        }
    }

    /// `true` for the two IEEE-754 binary32 float formats
    /// (`GrayF32Le` / `RgbF32Le`) — the Portable FloatMap members of the
    /// family. `false` for every integer format.
    pub fn is_float(self) -> bool {
        matches!(self, Self::GrayF32Le | Self::RgbF32Le)
    }

    /// `true` when the format carries an alpha channel
    /// (`Ya8` / `Ya16Le` / `Rgba` / `Bgra` / `Rgba64Le`).
    pub fn has_alpha(self) -> bool {
        matches!(
            self,
            Self::Ya8 | Self::Ya16Le | Self::Rgba | Self::Bgra | Self::Rgba64Le
        )
    }

    /// `true` when the format carries chroma (an RGB triple), i.e. it is
    /// not a grayscale / bilevel format.
    pub fn is_color(self) -> bool {
        matches!(
            self,
            Self::Rgb24 | Self::Rgb48Le | Self::RgbF32Le | Self::Rgba | Self::Bgra | Self::Rgba64Le
        )
    }

    /// `true` for the 1-bit layouts ([`PbmPixelFormat::MonoBlack`] /
    /// [`PbmPixelFormat::MonoWhite`]) — the only formats whose pixels
    /// are sub-byte.
    pub fn is_bilevel(self) -> bool {
        matches!(self, Self::MonoBlack | Self::MonoWhite)
    }
}

/// One image plane: row-major bytes plus the row stride in bytes.
///
/// Mirrors `oxideav_core::VideoPlane` so the registry-side conversion
/// is a trivial field-by-field copy. Every Netpbm layout is packed, so
/// a [`PbmImage`] has exactly one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Plane {
    /// Bytes per row in `data` (may exceed the logical row width when
    /// padding is required by the chosen pixel format).
    pub stride: usize,
    /// Raw plane bytes, packed `stride` × number of rows.
    pub data: Vec<u8>,
}

impl Plane {
    /// Wrap a plane buffer with its row stride.
    pub fn new(stride: usize, data: Vec<u8>) -> Self {
        Self { stride, data }
    }
}

/// Former name of [`Plane`].
#[deprecated(note = "renamed to oxideav_pbm::Plane (IMAGE_CRATE_API)")]
pub type PbmPlane = Plane;

/// Nominal sample range (H.273 `VideoFullRangeFlag`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ColorRange {
    /// No range was signalled.
    #[default]
    Unspecified,
    /// Limited (video / studio) range: `VideoFullRangeFlag == 0`.
    Limited,
    /// Full (PC) range: `VideoFullRangeFlag == 1`.
    Full,
}

/// Colour signalling of an image: the sample range plus the H.273
/// `ColourPrimaries` / `TransferCharacteristics` /
/// `MatrixCoefficients` code points (`2` = unspecified).
///
/// Netpbm files carry **no** colour signalling, so [`crate::decode`]
/// fills this with the family's documented convention:
/// [`ColorInfo::netpbm_default`] for the integer magics (full-range
/// RGB / grey in the BT.709 colour space and transfer the PPM format
/// text names) and [`ColorInfo::pfm_default`] for the Portable
/// FloatMap magics (full-range **linear** light, primaries
/// unspecified). The encoder cannot write any of it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ColorInfo {
    /// Sample range.
    pub range: ColorRange,
    /// H.273 `ColourPrimaries` code point (`1` = BT.709 / sRGB, `9` =
    /// BT.2020, `2` = unspecified).
    pub primaries: u8,
    /// H.273 `TransferCharacteristics` code point (`1` = BT.709, `8` =
    /// linear, `13` = sRGB, `2` = unspecified).
    pub transfer: u8,
    /// H.273 `MatrixCoefficients` code point (`0` = identity / RGB).
    pub matrix: u8,
}

impl ColorInfo {
    /// H.273 "unspecified" code point.
    pub const UNSPECIFIED: u8 = 2;
    /// H.273 `MatrixCoefficients` identity (RGB / GBR) code point.
    pub const MATRIX_IDENTITY: u8 = 0;
    /// H.273 `ColourPrimaries` BT.709 / sRGB code point.
    pub const PRIMARIES_BT709: u8 = 1;
    /// H.273 `TransferCharacteristics` BT.709 code point.
    pub const TRANSFER_BT709: u8 = 1;
    /// H.273 `TransferCharacteristics` linear code point.
    pub const TRANSFER_LINEAR: u8 = 8;
    /// H.273 `TransferCharacteristics` IEC 61966-2-1 sRGB code point.
    pub const TRANSFER_SRGB: u8 = 13;

    /// Build a description from its four parts.
    pub const fn new(range: ColorRange, primaries: u8, transfer: u8, matrix: u8) -> Self {
        Self {
            range,
            primaries,
            transfer,
            matrix,
        }
    }

    /// Every field unspecified.
    pub const fn unspecified() -> Self {
        Self::new(
            ColorRange::Unspecified,
            Self::UNSPECIFIED,
            Self::UNSPECIFIED,
            Self::UNSPECIFIED,
        )
    }

    /// The convention for the integer Netpbm magics (P1–P7): full-range
    /// RGB (`matrix` 0) in the BT.709 colour space with the BT.709
    /// transfer function — the colour space the PPM format text names
    /// (the staged family reference notes that in practice sRGB or
    /// linear data is also common and that nothing in the file says
    /// which).
    pub const fn netpbm_default() -> Self {
        Self::new(
            ColorRange::Full,
            Self::PRIMARIES_BT709,
            Self::TRANSFER_BT709,
            Self::MATRIX_IDENTITY,
        )
    }

    /// The convention for the Portable FloatMap magics (`Pf` / `PF`):
    /// full-range **linear** light (transfer 8), primaries unspecified,
    /// identity matrix — the reference describes PFM samples as linear
    /// light values with no encoding and names no primaries.
    pub const fn pfm_default() -> Self {
        Self::new(
            ColorRange::Full,
            Self::UNSPECIFIED,
            Self::TRANSFER_LINEAR,
            Self::MATRIX_IDENTITY,
        )
    }

    /// sRGB (IEC 61966-2-1): BT.709 primaries, sRGB transfer, identity
    /// matrix, full range.
    pub const fn srgb() -> Self {
        Self::new(
            ColorRange::Full,
            Self::PRIMARIES_BT709,
            Self::TRANSFER_SRGB,
            Self::MATRIX_IDENTITY,
        )
    }

    /// The documented default for a layout: [`ColorInfo::pfm_default`]
    /// for the float layouts, [`ColorInfo::netpbm_default`] otherwise.
    pub const fn default_for(format: PixelFormat) -> Self {
        if matches!(format, PixelFormat::GrayF32Le | PixelFormat::RgbF32Le) {
            Self::pfm_default()
        } else {
            Self::netpbm_default()
        }
    }

    /// Set the range.
    pub fn with_range(mut self, range: ColorRange) -> Self {
        self.range = range;
        self
    }

    /// Set the primaries code point.
    pub fn with_primaries(mut self, primaries: u8) -> Self {
        self.primaries = primaries;
        self
    }

    /// Set the transfer code point.
    pub fn with_transfer(mut self, transfer: u8) -> Self {
        self.transfer = transfer;
        self
    }

    /// Set the matrix code point.
    pub fn with_matrix(mut self, matrix: u8) -> Self {
        self.matrix = matrix;
        self
    }

    /// `true` when both primaries and transfer are specified (`!= 2`).
    pub fn is_specified(&self) -> bool {
        self.primaries != Self::UNSPECIFIED && self.transfer != Self::UNSPECIFIED
    }
}

impl Default for ColorInfo {
    /// [`ColorInfo::netpbm_default`].
    fn default() -> Self {
        Self::netpbm_default()
    }
}

/// The metadata blobs every image crate surfaces: an ICC profile, an
/// Exif payload, an XMP packet and a file gamma.
///
/// Netpbm has no metadata mechanism beyond free-text `#` header
/// comments (readable through [`crate::iter_pnm_header_comments`]), so
/// every field is `None` on a decoded image and the encoder cannot
/// carry whatever a caller sets. The type exists so [`PbmImage`] has
/// the same shape as every other image crate's image.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Metadata {
    /// ICC profile bytes. Always `None` from the decoder.
    pub icc: Option<Vec<u8>>,
    /// Exif payload. Always `None` from the decoder.
    pub exif: Option<Vec<u8>>,
    /// XMP packet. Always `None` from the decoder.
    pub xmp: Option<Vec<u8>>,
    /// File gamma as an encoding exponent. Always `None` from the
    /// decoder.
    pub gamma: Option<f32>,
}

impl Metadata {
    /// Empty metadata.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set (or clear) the ICC profile.
    pub fn with_icc(mut self, icc: impl Into<Option<Vec<u8>>>) -> Self {
        self.icc = icc.into();
        self
    }

    /// Set (or clear) the Exif payload.
    pub fn with_exif(mut self, exif: impl Into<Option<Vec<u8>>>) -> Self {
        self.exif = exif.into();
        self
    }

    /// Set (or clear) the XMP packet.
    pub fn with_xmp(mut self, xmp: impl Into<Option<Vec<u8>>>) -> Self {
        self.xmp = xmp.into();
        self
    }

    /// Set (or clear) the file gamma.
    pub fn with_gamma(mut self, gamma: impl Into<Option<f32>>) -> Self {
        self.gamma = gamma.into();
        self
    }

    /// `true` when no field is set.
    pub fn is_empty(&self) -> bool {
        self.icc.is_none() && self.exif.is_none() && self.xmp.is_none() && self.gamma.is_none()
    }
}

/// One decoded Netpbm image in its native layout, as returned by
/// [`crate::decode`] and consumed by [`crate::encode`].
///
/// `planes` holds exactly one packed plane (every Netpbm layout is
/// packed); `color` is the family's documented convention (the file
/// carries none); `metadata` is always empty (Netpbm has none). There
/// is no palette field: Netpbm has no indexed form.
///
/// Construct with [`PbmImage::new`] / [`PbmImage::packed`] /
/// [`PbmImage::from_rgb8`] / [`PbmImage::from_rgba8`], which validate
/// the plane geometry so an inconsistent image cannot exist and
/// [`PbmImage::to_rgb8`] / [`PbmImage::to_rgba8`] are infallible.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PbmImage {
    /// Picture width in pixels (≥ 1).
    pub width: u32,
    /// Picture height in pixels (≥ 1).
    pub height: u32,
    /// Pixel layout the plane carries.
    pub format: PixelFormat,
    /// One [`Plane`] per plane. Every Netpbm pixel format ships in a
    /// single packed plane, so this is always `len() == 1`.
    pub planes: Vec<Plane>,
    /// Colour signalling (range + H.273 code points) — the documented
    /// convention, since the file carries none.
    pub color: ColorInfo,
    /// ICC / Exif / XMP / gamma — always empty for Netpbm.
    pub metadata: Metadata,
}

impl PbmImage {
    /// Assemble an image from its geometry, layout and planes (exactly
    /// one for Netpbm). Colour is [`ColorInfo::default_for`] the layout
    /// and metadata empty; the `with_*` builders fill those in.
    ///
    /// Validates the geometry and returns [`PbmError::InvalidData`]
    /// when `width` or `height` is `0`, when there is not exactly one
    /// plane, when the plane's `stride` is below the layout's row size
    /// ([`PbmPixelFormat::row_bytes`]), or when its `data` is shorter
    /// than `stride × (height − 1) + row_bytes`.
    pub fn new(width: u32, height: u32, format: PixelFormat, planes: Vec<Plane>) -> Result<Self> {
        let img = Self {
            width,
            height,
            format,
            planes,
            color: ColorInfo::default_for(format),
            metadata: Metadata::default(),
        };
        img.validate()?;
        Ok(img)
    }

    /// One packed plane with an explicit row stride (`stride ≥` the
    /// layout's row size). Same validation as [`Self::new`].
    pub fn packed(
        width: u32,
        height: u32,
        format: PixelFormat,
        stride: usize,
        data: Vec<u8>,
    ) -> Result<Self> {
        Self::new(width, height, format, vec![Plane::new(stride, data)])
    }

    /// Tightly packed `Rgb24` from `3 × width × height` bytes (more is
    /// tolerated; fewer is [`PbmError::InvalidData`]). Encodes as P6.
    pub fn from_rgb8(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        let stride = (width as usize)
            .checked_mul(3)
            .ok_or_else(|| PbmError::invalid("Netpbm: row size overflows usize"))?;
        Self::packed(width, height, PixelFormat::Rgb24, stride, data)
    }

    /// Tightly packed `Rgba` from `4 × width × height` bytes (more is
    /// tolerated; fewer is [`PbmError::InvalidData`]). Encodes as P7
    /// `RGB_ALPHA`.
    pub fn from_rgba8(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        let stride = (width as usize)
            .checked_mul(4)
            .ok_or_else(|| PbmError::invalid("Netpbm: row size overflows usize"))?;
        Self::packed(width, height, PixelFormat::Rgba, stride, data)
    }

    /// Set the colour signalling. Informational only — Netpbm cannot
    /// carry it, so the encoder ignores it.
    pub fn with_color(mut self, color: ColorInfo) -> Self {
        self.color = color;
        self
    }

    /// Set the metadata. Netpbm cannot carry any of it; the encoder
    /// ignores it.
    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Image width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Native pixel layout.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Row stride in bytes of the pixel plane (`0` if there is none).
    pub fn stride(&self) -> usize {
        self.planes.first().map(|p| p.stride).unwrap_or(0)
    }

    /// Bytes per pixel of [`Self::format`] (`None` for the 1-bit
    /// layouts).
    pub fn bytes_per_pixel(&self) -> Option<usize> {
        self.format.bytes_per_pixel()
    }

    /// `true` when the layout carries alpha.
    pub fn has_alpha(&self) -> bool {
        self.format.has_alpha()
    }

    /// Bytes occupied by one row of the in-memory plane for this image's
    /// pixel format and width.
    ///
    /// For the bilevel layouts this is `width.div_ceil(8)`; for every
    /// other format it is `width * bytes_per_pixel`. This is the
    /// *minimum* contiguous row length — a plane's actual `stride` may
    /// be larger if the producer padded rows, so this value is a lower
    /// bound, not the stride. Saturates on overflow.
    pub fn min_row_bytes(&self) -> usize {
        self.format.row_bytes(self.width).unwrap_or(usize::MAX)
    }

    /// Minimum number of plane bytes a well-formed single-plane image of
    /// this width × height in this pixel format must contain
    /// (`min_row_bytes() * height`, saturating). A decoder fills exactly
    /// this many bytes; an encoder requires at least this many in the
    /// input plane.
    pub fn min_plane_len(&self) -> usize {
        self.min_row_bytes().saturating_mul(self.height as usize)
    }

    /// Validate that this image's single plane carries enough bytes for
    /// its declared `width` × `height` × pixel format, given the plane's
    /// own `stride`.
    ///
    /// Returns `Ok(())` when (a) there is exactly one plane, (b) both
    /// dimensions are non-zero, (c) the plane `stride` is at least
    /// [`PbmImage::min_row_bytes`], and (d) the plane `data` is long
    /// enough to hold `stride * (height - 1) + min_row_bytes` bytes (a
    /// producer may omit padding after the final row). [`Self::new`]
    /// runs this, so a decoder- or constructor-produced image always
    /// passes; a caller who mutated the public fields afterwards can
    /// re-check here.
    pub fn validate(&self) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(PbmError::invalid(
                "Netpbm: width and height must be non-zero",
            ));
        }
        if self.planes.len() != 1 {
            return Err(PbmError::invalid(format!(
                "Netpbm: expected exactly one packed plane, got {}",
                self.planes.len()
            )));
        }
        let min_row = self
            .format
            .row_bytes(self.width)
            .ok_or_else(|| PbmError::invalid("Netpbm: row size overflows usize"))?;
        let plane = &self.planes[0];
        if plane.stride < min_row {
            return Err(PbmError::invalid(format!(
                "Netpbm: stride {} below row size {min_row}",
                plane.stride
            )));
        }
        let need = plane
            .stride
            .checked_mul(self.height as usize - 1)
            .and_then(|n| n.checked_add(min_row))
            .ok_or_else(|| PbmError::invalid("Netpbm: plane size overflows usize"))?;
        if plane.data.len() < need {
            return Err(PbmError::invalid(format!(
                "Netpbm: plane holds {} bytes, geometry needs {need}",
                plane.data.len()
            )));
        }
        Ok(())
    }

    /// The pixel bytes — `Some` for every Netpbm image (all layouts are
    /// packed, one plane). Includes row padding when the plane's stride
    /// exceeds the row size.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.planes.first().map(|p| p.data.as_slice())
    }

    /// Consume the image and return its plane bytes (planes concatenated
    /// in order should there ever be more than one).
    pub fn into_raw(self) -> Vec<u8> {
        let mut planes = self.planes.into_iter();
        let mut out = planes.next().map(|p| p.data).unwrap_or_default();
        for p in planes {
            out.extend_from_slice(&p.data);
        }
        out
    }

    /// Pixel bytes of the single plane (empty if none).
    pub(crate) fn data(&self) -> &[u8] {
        self.as_bytes().unwrap_or(&[])
    }

    /// Tightly packed 8-bit RGBA, `4 × width` bytes per row, alpha `255`
    /// where the source has none. Exact integer kernels per layout:
    /// bilevel bits → 0 / 255 (`MonoBlack`: 1 = black), grey replicated
    /// to R = G = B, `Bgra` reordered, 16-bit samples reduced by
    /// round-half-up `(v × 255 + 32767) / 65535` (the same rule the
    /// decoder uses for non-natural `MAXVAL`s), float samples clamped to
    /// `[0, 1]` then `round(v × 255)` (NaN → 0). No colour management is
    /// applied.
    ///
    /// Infallible on any image produced by the decoder or a constructor;
    /// a plane too short for the geometry (after mutating the public
    /// fields) leaves the missing pixels zero.
    pub fn to_rgba8(&self) -> Vec<u8> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut out = vec![0u8; w.saturating_mul(h).saturating_mul(4)];
        let stride = self.stride();
        let src = self.data();
        let row_bytes = self.min_row_bytes();
        for y in 0..h {
            let Some(start) = y.checked_mul(stride) else {
                break;
            };
            let Some(row) = start
                .checked_add(row_bytes)
                .and_then(|end| src.get(start..end))
            else {
                break;
            };
            let dst = &mut out[y * w * 4..(y + 1) * w * 4];
            rgba_row(self.format, row, dst, w);
        }
        out
    }

    /// Tightly packed 8-bit RGB, `3 × width` bytes per row — the same
    /// kernels as [`Self::to_rgba8`] with alpha dropped (no compositing:
    /// a transparent pixel keeps its colour samples).
    pub fn to_rgb8(&self) -> Vec<u8> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut out = vec![0u8; w.saturating_mul(h).saturating_mul(3)];
        let stride = self.stride();
        let src = self.data();
        let row_bytes = self.min_row_bytes();
        let mut tmp = vec![0u8; w * 4];
        for y in 0..h {
            let Some(start) = y.checked_mul(stride) else {
                break;
            };
            let Some(row) = start
                .checked_add(row_bytes)
                .and_then(|end| src.get(start..end))
            else {
                break;
            };
            rgba_row(self.format, row, &mut tmp, w);
            let dst = &mut out[y * w * 3..(y + 1) * w * 3];
            for (s, d) in tmp.chunks_exact(4).zip(dst.chunks_exact_mut(3)) {
                d.copy_from_slice(&s[..3]);
            }
        }
        out
    }
}

/// Reduce a 16-bit sample to 8 bits by round-half-up
/// (`(v × 255 + 32767) / 65535`).
#[inline]
pub(crate) fn scale16_to_u8(v: u16) -> u8 {
    ((v as u32 * 255 + 32767) / 65535) as u8
}

/// Tone-scale a linear float sample to 8 bits: NaN → 0, clamp to
/// `[0, 1]`, `round(v × 255)`.
#[inline]
pub(crate) fn float_to_u8(v: f32) -> u8 {
    if v.is_nan() {
        return 0;
    }
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

#[inline]
fn le16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

#[inline]
fn lef32(b: &[u8]) -> f32 {
    f32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Convert one row of `w` pixels in `format` (at least the row's
/// minimum bytes in `row`) to RGBA8 in `dst` (`4 × w` bytes).
fn rgba_row(format: PixelFormat, row: &[u8], dst: &mut [u8], w: usize) {
    match format {
        PixelFormat::MonoBlack | PixelFormat::MonoWhite => {
            let one_is_black = format == PixelFormat::MonoBlack;
            for (x, px) in dst.chunks_exact_mut(4).enumerate().take(w) {
                let bit = (row[x / 8] >> (7 - (x % 8))) & 1 == 1;
                let v = if bit == one_is_black { 0 } else { 255 };
                px[0] = v;
                px[1] = v;
                px[2] = v;
                px[3] = 255;
            }
        }
        PixelFormat::Gray8 => {
            for (s, px) in row.iter().zip(dst.chunks_exact_mut(4)) {
                px[0] = *s;
                px[1] = *s;
                px[2] = *s;
                px[3] = 255;
            }
        }
        PixelFormat::Gray16Le => {
            for (s, px) in row.chunks_exact(2).zip(dst.chunks_exact_mut(4)) {
                let g = scale16_to_u8(le16(s));
                px[0] = g;
                px[1] = g;
                px[2] = g;
                px[3] = 255;
            }
        }
        PixelFormat::Ya8 => {
            for (s, px) in row.chunks_exact(2).zip(dst.chunks_exact_mut(4)) {
                px[0] = s[0];
                px[1] = s[0];
                px[2] = s[0];
                px[3] = s[1];
            }
        }
        PixelFormat::Ya16Le => {
            for (s, px) in row.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                let g = scale16_to_u8(le16(&s[0..2]));
                px[0] = g;
                px[1] = g;
                px[2] = g;
                px[3] = scale16_to_u8(le16(&s[2..4]));
            }
        }
        PixelFormat::Rgb24 => {
            for (s, px) in row.chunks_exact(3).zip(dst.chunks_exact_mut(4)) {
                px[..3].copy_from_slice(s);
                px[3] = 255;
            }
        }
        PixelFormat::Rgb48Le => {
            for (s, px) in row.chunks_exact(6).zip(dst.chunks_exact_mut(4)) {
                px[0] = scale16_to_u8(le16(&s[0..2]));
                px[1] = scale16_to_u8(le16(&s[2..4]));
                px[2] = scale16_to_u8(le16(&s[4..6]));
                px[3] = 255;
            }
        }
        PixelFormat::Rgba => {
            dst[..w * 4].copy_from_slice(&row[..w * 4]);
        }
        PixelFormat::Bgra => {
            for (s, px) in row.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                px[0] = s[2];
                px[1] = s[1];
                px[2] = s[0];
                px[3] = s[3];
            }
        }
        PixelFormat::Rgba64Le => {
            for (s, px) in row.chunks_exact(8).zip(dst.chunks_exact_mut(4)) {
                px[0] = scale16_to_u8(le16(&s[0..2]));
                px[1] = scale16_to_u8(le16(&s[2..4]));
                px[2] = scale16_to_u8(le16(&s[4..6]));
                px[3] = scale16_to_u8(le16(&s[6..8]));
            }
        }
        PixelFormat::GrayF32Le => {
            for (s, px) in row.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                let g = float_to_u8(lef32(s));
                px[0] = g;
                px[1] = g;
                px[2] = g;
                px[3] = 255;
            }
        }
        PixelFormat::RgbF32Le => {
            for (s, px) in row.chunks_exact(12).zip(dst.chunks_exact_mut(4)) {
                px[0] = float_to_u8(lef32(&s[0..4]));
                px[1] = float_to_u8(lef32(&s[4..8]));
                px[2] = float_to_u8(lef32(&s[8..12]));
                px[3] = 255;
            }
        }
    }
}

/// Tightly packed 8-bit RGB image: `width × height × 3` bytes,
/// row-major, no padding. What [`crate::decode_rgb8`] returns.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RgbImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width × height × 3` bytes, R, G, B per pixel.
    pub data: Vec<u8>,
}

impl RgbImage {
    /// Wrap a packed RGB buffer.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
        }
    }

    /// The pixel bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume and return the pixel bytes.
    pub fn into_raw(self) -> Vec<u8> {
        self.data
    }
}

/// Tightly packed 8-bit RGBA image: `width × height × 4` bytes,
/// row-major, no padding. What [`crate::decode_rgba8`] returns.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width × height × 4` bytes, R, G, B, A per pixel.
    pub data: Vec<u8>,
}

impl RgbaImage {
    /// Wrap a packed RGBA buffer.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
        }
    }

    /// The pixel bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume and return the pixel bytes.
    pub fn into_raw(self) -> Vec<u8> {
        self.data
    }
}

/// What [`crate::info`] reads from the header without decoding pixels.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ImageInfo {
    /// Width in pixels of the first image.
    pub width: u32,
    /// Height in pixels of the first image.
    pub height: u32,
    /// Native layout [`crate::decode`] would return for the first image.
    pub format: PixelFormat,
    /// Number of images concatenated in the file (≥ 1): the first image
    /// plus every well-formed image that follows it back-to-back (see
    /// [`crate::decode_all`]).
    pub frames: u32,
    /// `true` when `format` carries alpha.
    pub has_alpha: bool,
    /// Colour signalling — the documented convention for the magic.
    pub color: ColorInfo,
    /// Always `false` (Netpbm carries no ICC profile).
    pub has_icc: bool,
    /// Always `false` (Netpbm carries no Exif).
    pub has_exif: bool,
    /// Always `false` (Netpbm carries no XMP).
    pub has_xmp: bool,
    /// The fully parsed header of the first image: magic, `MAXVAL`,
    /// `DEPTH`, PAM `TUPLTYPE`, PFM byte order + scale, body offset.
    pub header: Header,
}

impl ImageInfo {
    /// The on-disk magic of the first image.
    pub fn magic(&self) -> crate::header::Magic {
        self.header.magic
    }

    /// The `MAXVAL` of the first image (`1` for P1 / P4, `0` for PFM).
    pub fn maxval(&self) -> u32 {
        self.header.maxval
    }

    /// `true` when the first image's body is plain-text (P1 / P2 / P3).
    pub fn is_ascii(&self) -> bool {
        self.header.magic.is_ascii()
    }
}

/// One image of a concatenated multi-image file, as returned by
/// [`crate::decode_all`]. Netpbm has no timing, so `delay` is always
/// `None`; `header` is that image's parsed header (`MAXVAL`, PAM
/// `TUPLTYPE`, PFM byte order + scale, …).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Frame {
    /// The decoded image in its native layout.
    pub image: PbmImage,
    /// Always `None` (Netpbm files carry no timing).
    pub delay: Option<Duration>,
    /// The parsed header of this image (its `data_offset` is relative to
    /// the start of this image, not of the stream).
    pub header: Header,
}

impl Frame {
    /// Wrap an image with its header.
    pub fn new(image: PbmImage, header: Header) -> Self {
        Self {
            image,
            delay: None,
            header,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_counts_match_format_family() {
        assert_eq!(PbmPixelFormat::MonoBlack.channels(), 1);
        assert_eq!(PbmPixelFormat::MonoWhite.channels(), 1);
        assert_eq!(PbmPixelFormat::Gray8.channels(), 1);
        assert_eq!(PbmPixelFormat::Gray16Le.channels(), 1);
        assert_eq!(PbmPixelFormat::GrayF32Le.channels(), 1);
        assert_eq!(PbmPixelFormat::Ya8.channels(), 2);
        assert_eq!(PbmPixelFormat::Ya16Le.channels(), 2);
        assert_eq!(PbmPixelFormat::Rgb24.channels(), 3);
        assert_eq!(PbmPixelFormat::Rgb48Le.channels(), 3);
        assert_eq!(PbmPixelFormat::RgbF32Le.channels(), 3);
        assert_eq!(PbmPixelFormat::Rgba.channels(), 4);
        assert_eq!(PbmPixelFormat::Bgra.channels(), 4);
        assert_eq!(PbmPixelFormat::Rgba64Le.channels(), 4);
    }

    #[test]
    fn bytes_per_pixel_is_none_only_for_bilevel() {
        assert_eq!(PbmPixelFormat::MonoBlack.bytes_per_pixel(), None);
        assert_eq!(PbmPixelFormat::MonoWhite.bytes_per_pixel(), None);
        let cases: &[(PbmPixelFormat, usize)] = &[
            (PbmPixelFormat::Gray8, 1),
            (PbmPixelFormat::Gray16Le, 2),
            (PbmPixelFormat::Ya8, 2),
            (PbmPixelFormat::Ya16Le, 4),
            (PbmPixelFormat::Rgb24, 3),
            (PbmPixelFormat::Rgb48Le, 6),
            (PbmPixelFormat::Rgba, 4),
            (PbmPixelFormat::Bgra, 4),
            (PbmPixelFormat::Rgba64Le, 8),
            (PbmPixelFormat::GrayF32Le, 4),
            (PbmPixelFormat::RgbF32Le, 12),
        ];
        for (f, want) in cases {
            assert_eq!(f.bytes_per_pixel(), Some(*want), "{f:?}");
            assert_eq!(*want, f.channels() * (f.bits_per_channel() / 8));
        }
        assert_eq!(PbmPixelFormat::MonoBlack.row_bytes(11), Some(2));
        assert_eq!(PbmPixelFormat::Rgb24.row_bytes(4), Some(12));
    }

    #[test]
    fn predicates_partition_the_layouts() {
        for f in PbmPixelFormat::ALL {
            assert_eq!(
                f.is_float(),
                matches!(f, PbmPixelFormat::GrayF32Le | PbmPixelFormat::RgbF32Le)
            );
            assert_eq!(
                f.is_bilevel(),
                matches!(f, PbmPixelFormat::MonoBlack | PbmPixelFormat::MonoWhite)
            );
            assert_eq!(f.has_alpha(), f.channels() == 2 || f.channels() == 4);
            assert_eq!(f.is_color(), f.channels() >= 3);
        }
    }

    #[test]
    #[allow(deprecated)]
    fn deprecated_float_aliases_resolve() {
        assert_eq!(PbmPixelFormat::GrayF32, PbmPixelFormat::GrayF32Le);
        assert_eq!(PbmPixelFormat::RgbF32, PbmPixelFormat::RgbF32Le);
    }

    #[test]
    fn new_validates_geometry() {
        assert!(PbmImage::packed(4, 2, PixelFormat::Rgb24, 12, vec![0u8; 24]).is_ok());
        // Last row may omit padding past min_row when stride > min_row.
        assert!(PbmImage::packed(3, 2, PixelFormat::Gray8, 8, vec![0u8; 11]).is_ok());
        // Plane too short.
        assert!(
            PbmImage::packed(4, 2, PixelFormat::Rgb24, 12, vec![0u8; 12])
                .unwrap_err()
                .is_invalid_data()
        );
        // Stride smaller than one packed row.
        assert!(PbmImage::packed(4, 1, PixelFormat::Rgb24, 6, vec![0u8; 12]).is_err());
        // Zero dimension.
        assert!(PbmImage::packed(0, 1, PixelFormat::Gray8, 0, vec![]).is_err());
        // Wrong plane count.
        assert!(PbmImage::new(2, 1, PixelFormat::Gray8, vec![]).is_err());
        assert!(PbmImage::new(
            2,
            1,
            PixelFormat::Gray8,
            vec![Plane::new(2, vec![0; 2]), Plane::new(2, vec![0; 2])]
        )
        .is_err());
        // from_rgb8 / from_rgba8 lengths.
        assert!(PbmImage::from_rgb8(2, 2, vec![0; 11]).is_err());
        assert!(PbmImage::from_rgb8(2, 2, vec![0; 12]).is_ok());
        assert!(PbmImage::from_rgba8(2, 2, vec![0; 16]).is_ok());
        // MonoBlack: 11 px → 2 bytes/row.
        let m = PbmImage::packed(11, 3, PixelFormat::MonoBlack, 2, vec![0u8; 6]).unwrap();
        assert_eq!(m.min_row_bytes(), 2);
        assert_eq!(m.min_plane_len(), 6);
    }

    #[test]
    fn color_defaults_follow_the_layout() {
        let g = PbmImage::packed(1, 1, PixelFormat::Gray8, 1, vec![0]).unwrap();
        assert_eq!(g.color, ColorInfo::netpbm_default());
        assert_eq!(g.color.transfer, ColorInfo::TRANSFER_BT709);
        let f = PbmImage::packed(1, 1, PixelFormat::GrayF32Le, 4, vec![0; 4]).unwrap();
        assert_eq!(f.color, ColorInfo::pfm_default());
        assert_eq!(f.color.transfer, ColorInfo::TRANSFER_LINEAR);
        assert!(g.metadata.is_empty());
    }

    #[test]
    fn to_rgba8_every_layout() {
        // MonoBlack: bits 1,0 → black, white.
        let m = PbmImage::packed(2, 1, PixelFormat::MonoBlack, 1, vec![0b1000_0000]).unwrap();
        assert_eq!(m.to_rgba8(), vec![0, 0, 0, 255, 255, 255, 255, 255]);
        assert_eq!(m.to_rgb8(), vec![0, 0, 0, 255, 255, 255]);
        let mw = PbmImage::packed(2, 1, PixelFormat::MonoWhite, 1, vec![0b1000_0000]).unwrap();
        assert_eq!(mw.to_rgb8(), vec![255, 255, 255, 0, 0, 0]);
        let g = PbmImage::packed(2, 1, PixelFormat::Gray8, 2, vec![7, 200]).unwrap();
        assert_eq!(g.to_rgba8(), vec![7, 7, 7, 255, 200, 200, 200, 255]);
        let g16 =
            PbmImage::packed(2, 1, PixelFormat::Gray16Le, 4, vec![0xFF, 0xFF, 0x00, 0x80]).unwrap();
        assert_eq!(g16.to_rgb8(), vec![255, 255, 255, 128, 128, 128]);
        let ya = PbmImage::packed(1, 1, PixelFormat::Ya8, 2, vec![9, 77]).unwrap();
        assert_eq!(ya.to_rgba8(), vec![9, 9, 9, 77]);
        let ya16 =
            PbmImage::packed(1, 1, PixelFormat::Ya16Le, 4, vec![0, 0x80, 0xFF, 0xFF]).unwrap();
        assert_eq!(ya16.to_rgba8(), vec![128, 128, 128, 255]);
        let rgb = PbmImage::from_rgb8(1, 1, vec![1, 2, 3]).unwrap();
        assert_eq!(rgb.to_rgba8(), vec![1, 2, 3, 255]);
        let rgb48 = PbmImage::packed(
            1,
            1,
            PixelFormat::Rgb48Le,
            6,
            vec![0, 0, 0, 0x80, 0xFF, 0xFF],
        )
        .unwrap();
        assert_eq!(rgb48.to_rgb8(), vec![0, 128, 255]);
        let rgba = PbmImage::from_rgba8(1, 1, vec![1, 2, 3, 4]).unwrap();
        assert_eq!(rgba.to_rgba8(), vec![1, 2, 3, 4]);
        assert_eq!(rgba.to_rgb8(), vec![1, 2, 3]);
        let bgra = PbmImage::packed(1, 1, PixelFormat::Bgra, 4, vec![1, 2, 3, 4]).unwrap();
        assert_eq!(bgra.to_rgba8(), vec![3, 2, 1, 4]);
        let rgba64 = PbmImage::packed(
            1,
            1,
            PixelFormat::Rgba64Le,
            8,
            vec![0xFF, 0xFF, 0, 0, 0, 0x80, 0x01, 0x00],
        )
        .unwrap();
        assert_eq!(rgba64.to_rgba8(), vec![255, 0, 128, 0]);
        let mut fdata = Vec::new();
        for v in [0.0f32, 0.5, 1.0, 2.0, -1.0, f32::NAN] {
            fdata.extend_from_slice(&v.to_le_bytes());
        }
        let gf = PbmImage::packed(6, 1, PixelFormat::GrayF32Le, 24, fdata.clone()).unwrap();
        let rgb = gf.to_rgb8();
        assert_eq!(
            rgb.chunks_exact(3).map(|p| p[0]).collect::<Vec<_>>(),
            vec![0, 128, 255, 255, 0, 0]
        );
        let rf = PbmImage::packed(2, 1, PixelFormat::RgbF32Le, 24, fdata).unwrap();
        assert_eq!(rf.to_rgba8(), vec![0, 128, 255, 255, 255, 0, 0, 255]);
    }

    #[test]
    fn to_rgba8_respects_stride_padding() {
        let g = PbmImage::packed(2, 2, PixelFormat::Gray8, 4, vec![1, 2, 99, 99, 3, 4]).unwrap();
        assert_eq!(g.to_rgb8(), vec![1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 4]);
    }

    #[test]
    fn to_rgba8_tolerates_a_mutated_short_plane() {
        let mut g = PbmImage::packed(2, 2, PixelFormat::Gray8, 2, vec![1, 2, 3, 4]).unwrap();
        g.planes[0].data.truncate(2);
        assert!(g.validate().is_err());
        assert_eq!(g.to_rgba8().len(), 16);
    }

    #[test]
    fn scale16_rounds_half_up() {
        assert_eq!(scale16_to_u8(0), 0);
        assert_eq!(scale16_to_u8(65535), 255);
        assert_eq!(scale16_to_u8(0x8000), 128);
        assert_eq!(scale16_to_u8(128), 0);
        assert_eq!(scale16_to_u8(129), 1);
        assert_eq!(float_to_u8(0.5), 128);
        assert_eq!(float_to_u8(f32::INFINITY), 255);
        assert_eq!(float_to_u8(f32::NEG_INFINITY), 0);
    }

    #[test]
    fn into_raw_and_as_bytes_expose_the_plane() {
        let img = PbmImage::from_rgb8(1, 1, vec![1, 2, 3]).unwrap();
        assert_eq!(img.as_bytes(), Some(&[1u8, 2, 3][..]));
        assert_eq!(img.stride(), 3);
        assert_eq!(img.into_raw(), vec![1, 2, 3]);
    }
}
