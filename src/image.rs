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
    /// 16-bit grayscale + alpha (`Y, A`), little-endian, 4 bytes per
    /// pixel. Decoded from / encoded to PAM `GRAYSCALE_ALPHA` at
    /// `MAXVAL` > 255. Like the two float-map variants below, it has no
    /// `oxideav_core::PixelFormat` counterpart yet, so the registry-side
    /// conversion returns `None` for it — the format is reachable
    /// through the standalone API and the crate-local [`PbmImage`]
    /// model.
    Ya16Le,
    /// Single-channel IEEE-754 binary32 (32-bit float) grayscale, one
    /// 4-byte sample per pixel. High-dynamic-range linear light. The
    /// plane stores each float in **little-endian** byte order (matching
    /// the crate's other `*Le` variants) regardless of the on-disk byte
    /// order it was read from. Decoded from / encoded to the `Pf`
    /// Portable FloatMap form (see [`crate::pfm`]).
    GrayF32,
    /// 3-channel (R, G, B interleaved) IEEE-754 binary32 (32-bit float)
    /// colour, 12 bytes per pixel. High-dynamic-range linear light. Each
    /// float is stored **little-endian** in the plane. Decoded from /
    /// encoded to the `PF` Portable FloatMap form (see [`crate::pfm`]).
    RgbF32,
}

impl PbmPixelFormat {
    /// Number of colour/alpha channels per pixel.
    ///
    /// * `MonoBlack` — 1 (a single packed bit plane).
    /// * `Gray8` / `Gray16Le` / `GrayF32` — 1.
    /// * `Ya8` / `Ya16Le` — 2 (luma + alpha).
    /// * `Rgb24` / `Rgb48Le` / `RgbF32` — 3.
    /// * `Rgba` / `Bgra` / `Rgba64Le` — 4.
    ///
    /// This is the logical channel count the on-disk Netpbm `DEPTH`
    /// (for PAM) or magic (for P1-P6 / `Pf` / `PF`) carries — it does
    /// not depend on the in-memory byte width of each channel.
    pub fn channels(self) -> usize {
        match self {
            Self::MonoBlack | Self::Gray8 | Self::Gray16Le | Self::GrayF32 => 1,
            Self::Ya8 | Self::Ya16Le => 2,
            Self::Rgb24 | Self::Rgb48Le | Self::RgbF32 => 3,
            Self::Rgba | Self::Bgra | Self::Rgba64Le => 4,
        }
    }

    /// Bits per channel held **in memory** for one decoded pixel.
    ///
    /// * `MonoBlack` — 1 (one packed bit per pixel).
    /// * the `*8` / `Rgb24` / `Rgba` / `Bgra` integer formats — 8.
    /// * the `*16Le` integer formats — 16.
    /// * the `*F32` float formats — 32.
    ///
    /// Note this reports the *in-memory* sample width, which for the
    /// `*Le` and `*F32` variants is the crate's canonical little-endian
    /// plane layout regardless of the on-disk byte order the image was
    /// read from.
    pub fn bits_per_channel(self) -> usize {
        match self {
            Self::MonoBlack => 1,
            Self::Gray8 | Self::Rgb24 | Self::Rgba | Self::Bgra | Self::Ya8 => 8,
            Self::Gray16Le | Self::Rgb48Le | Self::Rgba64Le | Self::Ya16Le => 16,
            Self::GrayF32 | Self::RgbF32 => 32,
        }
    }

    /// Bytes occupied by one pixel in the in-memory plane.
    ///
    /// Returns `None` for [`PbmPixelFormat::MonoBlack`], whose pixels
    /// are sub-byte (1 bit each, MSB-first packed with rows padded to a
    /// byte boundary) — a single pixel does not occupy a whole number of
    /// bytes, so the byte-per-row stride is `width.div_ceil(8)` rather
    /// than `width * bytes_per_pixel`. Every other format packs a whole
    /// number of bytes per pixel (`channels * bits_per_channel / 8`).
    pub fn bytes_per_pixel(self) -> Option<usize> {
        if self == Self::MonoBlack {
            return None;
        }
        Some(self.channels() * (self.bits_per_channel() / 8))
    }

    /// `true` for the two IEEE-754 binary32 float formats
    /// (`GrayF32` / `RgbF32`) — the Portable FloatMap members of the
    /// family. `false` for every integer format.
    pub fn is_float(self) -> bool {
        matches!(self, Self::GrayF32 | Self::RgbF32)
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
    /// not a grayscale / bilevel format. The `Rgb*` and `Rgba` / `Bgra`
    /// / `Rgba64Le` formats are colour; `MonoBlack`, the `Gray*`, and
    /// the `Ya*` luma+alpha formats are not.
    pub fn is_color(self) -> bool {
        matches!(
            self,
            Self::Rgb24 | Self::Rgb48Le | Self::RgbF32 | Self::Rgba | Self::Bgra | Self::Rgba64Le
        )
    }

    /// `true` for the single bilevel (1-bit) format,
    /// [`PbmPixelFormat::MonoBlack`] — the only format whose pixels are
    /// sub-byte. A convenience predicate so callers branching on the
    /// packed-bit plane layout don't have to name the variant directly.
    pub fn is_bilevel(self) -> bool {
        self == Self::MonoBlack
    }
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

impl PbmImage {
    /// Bytes occupied by one row of the in-memory plane for this image's
    /// pixel format and width.
    ///
    /// For [`PbmPixelFormat::MonoBlack`] (1-bit packed) this is
    /// `width.div_ceil(8)`; for every other format it is
    /// `width * bytes_per_pixel`. This is the *minimum* contiguous row
    /// length — a plane's actual `stride` may be larger if the producer
    /// padded rows, so this value is a lower bound, not the stride.
    pub fn min_row_bytes(&self) -> usize {
        let w = self.width as usize;
        match self.pixel_format.bytes_per_pixel() {
            Some(bpp) => w * bpp,
            // MonoBlack: 1 bit per pixel, rows padded to a byte boundary.
            None => w.div_ceil(8),
        }
    }

    /// Minimum number of plane bytes a well-formed single-plane image of
    /// this width × height in this pixel format must contain
    /// (`min_row_bytes() * height`). A decoder fills exactly this many
    /// bytes; an encoder requires at least this many in the input plane.
    pub fn min_plane_len(&self) -> usize {
        self.min_row_bytes() * self.height as usize
    }

    /// Validate that this image's single plane carries enough bytes for
    /// its declared `width` × `height` × pixel format, given the plane's
    /// own `stride`.
    ///
    /// Returns `Ok(())` when (a) there is exactly one plane, (b) the
    /// plane `stride` is at least [`PbmImage::min_row_bytes`], and (c)
    /// the plane `data` is long enough to hold `stride * height` bytes
    /// (or, for the last row, at least `min_row_bytes`). This is a
    /// crate-local consistency check callers can run on a
    /// programmatically-built image before handing it to the encoder; the
    /// decoder always produces images that pass it.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.planes.len() != 1 {
            return Err("PbmImage must have exactly one plane");
        }
        if self.width == 0 || self.height == 0 {
            return Err("PbmImage width and height must be non-zero");
        }
        let plane = &self.planes[0];
        let min_row = self.min_row_bytes();
        if plane.stride < min_row {
            return Err("PbmImage plane stride is smaller than one packed row");
        }
        let h = self.height as usize;
        // The last row only needs `min_row` bytes (a producer may omit
        // padding after the final row); every preceding row needs a full
        // `stride`.
        let need = if h == 0 {
            0
        } else {
            plane.stride * (h - 1) + min_row
        };
        if plane.data.len() < need {
            return Err("PbmImage plane data is shorter than width × height demands");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_counts_match_format_family() {
        assert_eq!(PbmPixelFormat::MonoBlack.channels(), 1);
        assert_eq!(PbmPixelFormat::Gray8.channels(), 1);
        assert_eq!(PbmPixelFormat::Gray16Le.channels(), 1);
        assert_eq!(PbmPixelFormat::GrayF32.channels(), 1);
        assert_eq!(PbmPixelFormat::Ya8.channels(), 2);
        assert_eq!(PbmPixelFormat::Ya16Le.channels(), 2);
        assert_eq!(PbmPixelFormat::Rgb24.channels(), 3);
        assert_eq!(PbmPixelFormat::Rgb48Le.channels(), 3);
        assert_eq!(PbmPixelFormat::RgbF32.channels(), 3);
        assert_eq!(PbmPixelFormat::Rgba.channels(), 4);
        assert_eq!(PbmPixelFormat::Bgra.channels(), 4);
        assert_eq!(PbmPixelFormat::Rgba64Le.channels(), 4);
    }

    #[test]
    fn bits_per_channel_match_format_family() {
        assert_eq!(PbmPixelFormat::MonoBlack.bits_per_channel(), 1);
        for f in [
            PbmPixelFormat::Gray8,
            PbmPixelFormat::Rgb24,
            PbmPixelFormat::Rgba,
            PbmPixelFormat::Bgra,
            PbmPixelFormat::Ya8,
        ] {
            assert_eq!(f.bits_per_channel(), 8, "{f:?}");
        }
        for f in [
            PbmPixelFormat::Gray16Le,
            PbmPixelFormat::Rgb48Le,
            PbmPixelFormat::Rgba64Le,
            PbmPixelFormat::Ya16Le,
        ] {
            assert_eq!(f.bits_per_channel(), 16, "{f:?}");
        }
        assert_eq!(PbmPixelFormat::GrayF32.bits_per_channel(), 32);
        assert_eq!(PbmPixelFormat::RgbF32.bits_per_channel(), 32);
    }

    #[test]
    fn bytes_per_pixel_is_none_only_for_monoblack() {
        assert_eq!(PbmPixelFormat::MonoBlack.bytes_per_pixel(), None);
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
            (PbmPixelFormat::GrayF32, 4),
            (PbmPixelFormat::RgbF32, 12),
        ];
        for (f, want) in cases {
            assert_eq!(f.bytes_per_pixel(), Some(*want), "{f:?}");
            // Cross-check the derivation: bpp == channels * bytes/channel.
            assert_eq!(*want, f.channels() * (f.bits_per_channel() / 8));
        }
    }

    #[test]
    fn float_predicate_covers_only_the_two_floatmap_formats() {
        assert!(PbmPixelFormat::GrayF32.is_float());
        assert!(PbmPixelFormat::RgbF32.is_float());
        for f in [
            PbmPixelFormat::MonoBlack,
            PbmPixelFormat::Gray8,
            PbmPixelFormat::Gray16Le,
            PbmPixelFormat::Rgb24,
            PbmPixelFormat::Rgb48Le,
            PbmPixelFormat::Rgba,
            PbmPixelFormat::Bgra,
            PbmPixelFormat::Rgba64Le,
            PbmPixelFormat::Ya8,
            PbmPixelFormat::Ya16Le,
        ] {
            assert!(!f.is_float(), "{f:?}");
        }
    }

    #[test]
    fn alpha_predicate_covers_the_alpha_formats() {
        for f in [
            PbmPixelFormat::Ya8,
            PbmPixelFormat::Ya16Le,
            PbmPixelFormat::Rgba,
            PbmPixelFormat::Bgra,
            PbmPixelFormat::Rgba64Le,
        ] {
            assert!(f.has_alpha(), "{f:?}");
        }
        for f in [
            PbmPixelFormat::MonoBlack,
            PbmPixelFormat::Gray8,
            PbmPixelFormat::Gray16Le,
            PbmPixelFormat::GrayF32,
            PbmPixelFormat::Rgb24,
            PbmPixelFormat::Rgb48Le,
            PbmPixelFormat::RgbF32,
        ] {
            assert!(!f.has_alpha(), "{f:?}");
        }
    }

    #[test]
    fn color_and_bilevel_predicates_partition_correctly() {
        for f in [
            PbmPixelFormat::Rgb24,
            PbmPixelFormat::Rgb48Le,
            PbmPixelFormat::RgbF32,
            PbmPixelFormat::Rgba,
            PbmPixelFormat::Bgra,
            PbmPixelFormat::Rgba64Le,
        ] {
            assert!(f.is_color(), "{f:?}");
            assert!(!f.is_bilevel(), "{f:?}");
        }
        // MonoBlack is the only bilevel format and is not "color".
        assert!(PbmPixelFormat::MonoBlack.is_bilevel());
        assert!(!PbmPixelFormat::MonoBlack.is_color());
        // Gray / Ya are neither color nor bilevel.
        for f in [
            PbmPixelFormat::Gray8,
            PbmPixelFormat::Gray16Le,
            PbmPixelFormat::GrayF32,
            PbmPixelFormat::Ya8,
            PbmPixelFormat::Ya16Le,
        ] {
            assert!(!f.is_color(), "{f:?}");
            assert!(!f.is_bilevel(), "{f:?}");
        }
    }

    fn img(fmt: PbmPixelFormat, w: u32, h: u32, stride: usize, data: Vec<u8>) -> PbmImage {
        PbmImage {
            width: w,
            height: h,
            pixel_format: fmt,
            planes: vec![PbmPlane { stride, data }],
            pts: None,
        }
    }

    #[test]
    fn min_row_bytes_handles_monoblack_padding() {
        // 11 px MonoBlack → 2 bytes/row (last 5 bits padding).
        let m = img(PbmPixelFormat::MonoBlack, 11, 3, 2, vec![0u8; 6]);
        assert_eq!(m.min_row_bytes(), 2);
        assert_eq!(m.min_plane_len(), 6);
        // 8 px MonoBlack → exactly 1 byte/row.
        let m8 = img(PbmPixelFormat::MonoBlack, 8, 1, 1, vec![0u8; 1]);
        assert_eq!(m8.min_row_bytes(), 1);
        // Rgb24 width 4 → 12 bytes/row.
        let c = img(PbmPixelFormat::Rgb24, 4, 2, 12, vec![0u8; 24]);
        assert_eq!(c.min_row_bytes(), 12);
        assert_eq!(c.min_plane_len(), 24);
    }

    #[test]
    fn validate_accepts_well_formed_image() {
        let c = img(PbmPixelFormat::Rgb24, 4, 2, 12, vec![0u8; 24]);
        assert_eq!(c.validate(), Ok(()));
        // Last row may omit padding past min_row when stride > min_row.
        let padded = img(PbmPixelFormat::Gray8, 3, 2, 8, vec![0u8; 8 + 3]);
        assert_eq!(padded.validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_short_plane_and_bad_stride() {
        // Plane too short for declared dimensions.
        let short = img(PbmPixelFormat::Rgb24, 4, 2, 12, vec![0u8; 12]);
        assert!(short.validate().is_err());
        // Stride smaller than one packed row.
        let bad_stride = img(PbmPixelFormat::Rgb24, 4, 1, 6, vec![0u8; 12]);
        assert!(bad_stride.validate().is_err());
        // Zero dimension.
        let zero = img(PbmPixelFormat::Gray8, 0, 1, 0, vec![]);
        assert!(zero.validate().is_err());
        // Wrong plane count.
        let mut multi = img(PbmPixelFormat::Gray8, 2, 1, 2, vec![0u8; 2]);
        multi.planes.push(PbmPlane {
            stride: 2,
            data: vec![0u8; 2],
        });
        assert!(multi.validate().is_err());
    }
}
