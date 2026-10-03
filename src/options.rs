//! Decode-side limits ([`DecodeOptions`]) and encode-side knobs
//! ([`EncodeOptions`]) of the standalone API.

use crate::error::{PbmError, Result};

/// Limits and strictness for [`crate::decode_with`] /
/// [`crate::decode_all_with`].
///
/// Every limit is checked against the parsed header **before** any
/// pixel buffer is allocated, so a hostile header fails with
/// [`PbmError::LimitExceeded`] instead of committing memory. The
/// defaults are: no dimension / pixel-count limit, decoded plane
/// capped at [`DecodeOptions::DEFAULT_MAX_BYTES`] (1 GiB), `strict =
/// false`.
///
/// `strict` governs the one thing the family leaves to the
/// implementation: a sample larger than the header's `MAXVAL` (P2 / P3
/// plain text, or a binary P5 / P6 / P7 body at a `MAXVAL` below the
/// natural 255 / 65535). Lenient mode clamps it to `MAXVAL` (the
/// long-standing decoder behaviour, so decoded bytes are unchanged);
/// strict mode rejects the image with [`PbmError::InvalidData`]. Every
/// structural rule — magic, dimensions, `MAXVAL` / `DEPTH` ranges, PAM
/// `TUPLTYPE`-vs-`DEPTH` consistency, body length, the PFM header
/// grammar — is enforced in both modes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeOptions {
    /// Reject images wider than this (pixels).
    pub max_width: Option<u32>,
    /// Reject images taller than this (pixels).
    pub max_height: Option<u32>,
    /// Reject images with more than this many pixels (`width ×
    /// height`).
    pub max_pixels: Option<u64>,
    /// Reject images whose decoded plane would exceed this many bytes
    /// (`height × row bytes` of the native layout).
    pub max_bytes: Option<u64>,
    /// Reject (instead of clamping) samples above `MAXVAL`.
    pub strict: bool,
}

impl DecodeOptions {
    /// Default [`Self::max_bytes`]: 1 GiB of decoded plane.
    pub const DEFAULT_MAX_BYTES: u64 = 1 << 30;

    /// The defaults (see the type docs).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set (or lift with `None`) the width limit.
    pub fn with_max_width(mut self, max_width: impl Into<Option<u32>>) -> Self {
        self.max_width = max_width.into();
        self
    }

    /// Set (or lift with `None`) the height limit.
    pub fn with_max_height(mut self, max_height: impl Into<Option<u32>>) -> Self {
        self.max_height = max_height.into();
        self
    }

    /// Set (or lift with `None`) the pixel-count limit.
    pub fn with_max_pixels(mut self, max_pixels: impl Into<Option<u64>>) -> Self {
        self.max_pixels = max_pixels.into();
        self
    }

    /// Set (or lift with `None`) the decoded-bytes limit.
    pub fn with_max_bytes(mut self, max_bytes: impl Into<Option<u64>>) -> Self {
        self.max_bytes = max_bytes.into();
        self
    }

    /// Set strict mode (see the type docs).
    pub fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Lift every limit (`max_*` all `None`).
    pub fn unlimited(mut self) -> Self {
        self.max_width = None;
        self.max_height = None;
        self.max_pixels = None;
        self.max_bytes = None;
        self
    }

    /// Check a header's geometry against the limits. `bytes` is the
    /// decoded plane size the native layout implies.
    pub(crate) fn check(&self, width: u32, height: u32, bytes: u64) -> Result<()> {
        if let Some(m) = self.max_width {
            if width > m {
                return Err(PbmError::limit(format!(
                    "Netpbm: width {width} exceeds max_width {m}"
                )));
            }
        }
        if let Some(m) = self.max_height {
            if height > m {
                return Err(PbmError::limit(format!(
                    "Netpbm: height {height} exceeds max_height {m}"
                )));
            }
        }
        let pixels = u64::from(width) * u64::from(height);
        if let Some(m) = self.max_pixels {
            if pixels > m {
                return Err(PbmError::limit(format!(
                    "Netpbm: {pixels} pixels exceed max_pixels {m}"
                )));
            }
        }
        if let Some(m) = self.max_bytes {
            if bytes > m {
                return Err(PbmError::limit(format!(
                    "Netpbm: decoded plane of {bytes} bytes exceeds max_bytes {m}"
                )));
            }
        }
        Ok(())
    }
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            max_width: None,
            max_height: None,
            max_pixels: None,
            max_bytes: Some(Self::DEFAULT_MAX_BYTES),
            strict: false,
        }
    }
}

/// Output flavour for [`crate::encode`] / [`crate::encode_rgb8`] /
/// [`crate::encode_rgba8`] / [`crate::encode_to`].
///
/// The default (`EncodeOptions::default()`) is the **raw, natural
/// flavour for the layout**: `MonoBlack` / `MonoWhite` → P4,
/// `Gray8` / `Gray16Le` → P5, `Rgb24` / `Rgb48Le` → P6, every alpha
/// layout → P7 PAM (`GRAYSCALE_ALPHA` / `RGB_ALPHA`), the float layouts
/// → Portable FloatMap (`Pf` / `PF`, little-endian, scale 1). The
/// natural `MAXVAL` is 255 for 8-bit and 65535 for 16-bit samples.
///
/// Every field is a behaviour knob (never a function suffix):
///
/// | Field | Effect |
/// |---|---|
/// | `ascii` | plain-text body: P1 / P2 / P3 for the bilevel / grey / RGB layouts; `Unsupported` for alpha, float, or with `pam` / `tupltype` |
/// | `pam` | force the P7 PAM container (`BLACKANDWHITE` / `GRAYSCALE` / `RGB` / `GRAYSCALE_ALPHA` / `RGB_ALPHA`) for any integer layout |
/// | `maxval` | override `MAXVAL` (1..=65535); samples are rescaled by round-half-up to the new range (1 byte on disk when ≤ 255, else 2 big-endian) |
/// | `tupltype` | custom PAM `TUPLTYPE` token (implies `pam`); `DEPTH` still follows the layout |
/// | `pfm_little_endian` | PFM byte order (sign of the scale line); default `true` |
/// | `pfm_scale` | PFM scale-line magnitude written as advisory metadata (samples unchanged); default `1.0` |
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct EncodeOptions {
    /// Emit the plain-text (ASCII) form P1 / P2 / P3 instead of the raw
    /// form. `Unsupported` for layouts with no plain form (alpha, float).
    pub ascii: bool,
    /// Emit the P7 PAM container even for layouts that have a classic
    /// P4 / P5 / P6 form. Not combinable with `ascii`.
    pub pam: bool,
    /// Override the on-disk `MAXVAL` (1..=65535). `None` = the layout's
    /// natural value (255 / 65535). Ignored by the bilevel and float
    /// layouts (P1 / P4 have no `MAXVAL`; PAM `BLACKANDWHITE` is always
    /// `MAXVAL 1`; PFM has none).
    pub maxval: Option<u32>,
    /// Custom PAM `TUPLTYPE` token (no whitespace; implies `pam`).
    pub tupltype: Option<String>,
    /// Portable FloatMap byte order: `true` = little-endian (negative
    /// scale line), `false` = big-endian.
    pub pfm_little_endian: bool,
    /// Portable FloatMap scale-line magnitude (finite, non-zero). Written
    /// as advisory metadata; the samples are stored verbatim.
    pub pfm_scale: f32,
}

impl EncodeOptions {
    /// The defaults (raw, natural flavour; see the type docs).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set [`Self::ascii`].
    pub fn with_ascii(mut self, ascii: bool) -> Self {
        self.ascii = ascii;
        self
    }

    /// Set [`Self::pam`].
    pub fn with_pam(mut self, pam: bool) -> Self {
        self.pam = pam;
        self
    }

    /// Set (or clear) [`Self::maxval`].
    pub fn with_maxval(mut self, maxval: impl Into<Option<u32>>) -> Self {
        self.maxval = maxval.into();
        self
    }

    /// Set (or clear) [`Self::tupltype`].
    pub fn with_tupltype(mut self, tupltype: impl Into<Option<String>>) -> Self {
        self.tupltype = tupltype.into();
        self
    }

    /// Set [`Self::pfm_little_endian`].
    pub fn with_pfm_little_endian(mut self, little_endian: bool) -> Self {
        self.pfm_little_endian = little_endian;
        self
    }

    /// Set [`Self::pfm_scale`].
    pub fn with_pfm_scale(mut self, scale: f32) -> Self {
        self.pfm_scale = scale;
        self
    }

    /// Validate the field combination independent of any image.
    pub(crate) fn validate(&self) -> Result<()> {
        if self.ascii && (self.pam || self.tupltype.is_some()) {
            return Err(PbmError::unsupported(
                "Netpbm encoder: PAM (P7) has no plain-text form; `ascii` cannot be combined with `pam` / `tupltype`",
            ));
        }
        if let Some(mv) = self.maxval {
            if !(1..=65535).contains(&mv) {
                return Err(PbmError::invalid(format!(
                    "Netpbm encoder: maxval {mv} outside 1..=65535"
                )));
            }
        }
        if let Some(t) = &self.tupltype {
            if t.is_empty() || t.bytes().any(|b| b.is_ascii_whitespace() || b == b'#') {
                return Err(PbmError::invalid(
                    "Netpbm encoder: TUPLTYPE must be a non-empty token without whitespace or '#'",
                ));
            }
        }
        if !self.pfm_scale.is_finite() || self.pfm_scale == 0.0 {
            return Err(PbmError::invalid(
                "Netpbm encoder: pfm_scale must be finite and non-zero",
            ));
        }
        Ok(())
    }
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            ascii: false,
            pam: false,
            maxval: None,
            tupltype: None,
            pfm_little_endian: true,
            pfm_scale: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_defaults_are_finite_bytes_only() {
        let d = DecodeOptions::default();
        assert_eq!(d.max_width, None);
        assert_eq!(d.max_height, None);
        assert_eq!(d.max_pixels, None);
        assert_eq!(d.max_bytes, Some(1 << 30));
        assert!(!d.strict);
        let u = d.clone().unlimited();
        assert_eq!(u.max_bytes, None);
        assert!(d.check(10, 10, 300).is_ok());
        assert!(DecodeOptions::new()
            .with_max_width(4)
            .check(5, 1, 5)
            .unwrap_err()
            .is_limit_exceeded());
        assert!(DecodeOptions::new()
            .with_max_height(4)
            .check(1, 5, 5)
            .is_err());
        assert!(DecodeOptions::new()
            .with_max_pixels(10u64)
            .check(4, 4, 16)
            .is_err());
        assert!(DecodeOptions::new()
            .with_max_bytes(10u64)
            .check(4, 4, 16)
            .is_err());
    }

    #[test]
    fn encode_option_combinations_are_validated() {
        assert!(EncodeOptions::default().validate().is_ok());
        assert!(EncodeOptions::new()
            .with_ascii(true)
            .with_pam(true)
            .validate()
            .unwrap_err()
            .is_unsupported());
        assert!(EncodeOptions::new()
            .with_ascii(true)
            .with_tupltype("X".to_string())
            .validate()
            .is_err());
        assert!(EncodeOptions::new().with_maxval(0).validate().is_err());
        assert!(EncodeOptions::new().with_maxval(70000).validate().is_err());
        assert!(EncodeOptions::new()
            .with_tupltype("HAS SPACE".to_string())
            .validate()
            .is_err());
        assert!(EncodeOptions::new()
            .with_tupltype(String::new())
            .validate()
            .is_err());
        assert!(EncodeOptions::new().with_pfm_scale(0.0).validate().is_err());
        assert!(EncodeOptions::new()
            .with_pfm_scale(f32::NAN)
            .validate()
            .is_err());
        assert!(EncodeOptions::new()
            .with_pam(true)
            .with_maxval(1000)
            .with_tupltype("MY_TYPE".to_string())
            .with_pfm_little_endian(false)
            .with_pfm_scale(2.0)
            .validate()
            .is_ok());
    }
}
