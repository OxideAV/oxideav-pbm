//! `oxideav-core` integration layer for `oxideav-pbm`.
//!
//! Gated behind the default-on `registry` feature so image-library
//! consumers can depend on `oxideav-pbm` with `default-features = false`
//! and skip the `oxideav-core` dependency entirely.
//!
//! The module exposes:
//! * [`register`] / [`register_codecs`] / [`register_containers`] — the
//!   `RuntimeContext` / `CodecRegistry` / `ContainerRegistry` entry
//!   points the umbrella `oxideav` crate calls during framework
//!   initialisation.
//! * The frame bridge: `From<PbmImage> for VideoFrame` and
//!   [`PbmImage::from_video_frame`] / `TryFrom<(&VideoFrame,
//!   &CodecParameters)>`, plus the 1:1 [`PbmPixelFormat`] ↔
//!   [`oxideav_core::PixelFormat`] mapping ([`to_core_pixel_format`] /
//!   [`from_core_pixel_format`]). Netpbm files carry no colour
//!   signalling, so no colour-signal side-channel is stamped.
//! * The `From<PbmError> for oxideav_core::Error` conversion that lets
//!   the trait-side `Decoder` / `Encoder` impls (living in
//!   `decoder.rs` / `encoder.rs`) bubble errors up through the
//!   framework error type, and the `CodecOptionsStruct` schema for
//!   [`EncodeOptions`].

use oxideav_core::{
    CodecCapabilities, CodecId, CodecInfo, CodecOptionsStruct, CodecParameters, CodecRegistry,
    ContainerRegistry, OptionField, OptionKind, OptionValue, PixelFormat, RuntimeContext,
    VideoFrame, VideoPlane,
};

use crate::container;
use crate::error::PbmError;
use crate::image::{PbmImage, PbmPixelFormat};
use crate::options::EncodeOptions;

/// Convert a [`PbmError`] into the framework-shared `oxideav_core::Error`
/// so trait impls in this crate can use `?` on errors returned by the
/// framework-free decode/encode functions.
impl From<PbmError> for oxideav_core::Error {
    fn from(e: PbmError) -> Self {
        match e {
            PbmError::InvalidData(s) => oxideav_core::Error::InvalidData(s),
            PbmError::Unsupported(s) => oxideav_core::Error::Unsupported(s),
            PbmError::LimitExceeded(s) => oxideav_core::Error::ResourceExhausted(s),
            PbmError::Io(e) => oxideav_core::Error::Io(e),
        }
    }
}

// ---- pixel formats --------------------------------------------------------

/// The 1:1 name mapping from [`PbmPixelFormat`] to the framework enum.
pub fn to_core_pixel_format(f: PbmPixelFormat) -> PixelFormat {
    match f {
        PbmPixelFormat::MonoBlack => PixelFormat::MonoBlack,
        PbmPixelFormat::MonoWhite => PixelFormat::MonoWhite,
        PbmPixelFormat::Gray8 => PixelFormat::Gray8,
        PbmPixelFormat::Gray16Le => PixelFormat::Gray16Le,
        PbmPixelFormat::Ya8 => PixelFormat::Ya8,
        PbmPixelFormat::Ya16Le => PixelFormat::Ya16Le,
        PbmPixelFormat::Rgb24 => PixelFormat::Rgb24,
        PbmPixelFormat::Rgb48Le => PixelFormat::Rgb48Le,
        PbmPixelFormat::Rgba => PixelFormat::Rgba,
        PbmPixelFormat::Bgra => PixelFormat::Bgra,
        PbmPixelFormat::Rgba64Le => PixelFormat::Rgba64Le,
        PbmPixelFormat::GrayF32Le => PixelFormat::GrayF32Le,
        PbmPixelFormat::RgbF32Le => PixelFormat::RgbF32Le,
    }
}

/// Map a framework pixel format to [`PbmPixelFormat`]; `Err` for the
/// layouts the Netpbm family cannot carry.
pub fn from_core_pixel_format(f: PixelFormat) -> crate::Result<PbmPixelFormat> {
    Ok(match f {
        PixelFormat::MonoBlack => PbmPixelFormat::MonoBlack,
        PixelFormat::MonoWhite => PbmPixelFormat::MonoWhite,
        PixelFormat::Gray8 => PbmPixelFormat::Gray8,
        PixelFormat::Gray16Le => PbmPixelFormat::Gray16Le,
        PixelFormat::Ya8 => PbmPixelFormat::Ya8,
        PixelFormat::Ya16Le => PbmPixelFormat::Ya16Le,
        PixelFormat::Rgb24 => PbmPixelFormat::Rgb24,
        PixelFormat::Rgb48Le => PbmPixelFormat::Rgb48Le,
        PixelFormat::Rgba => PbmPixelFormat::Rgba,
        PixelFormat::Bgra => PbmPixelFormat::Bgra,
        PixelFormat::Rgba64Le => PbmPixelFormat::Rgba64Le,
        PixelFormat::GrayF32Le => PbmPixelFormat::GrayF32Le,
        PixelFormat::RgbF32Le => PbmPixelFormat::RgbF32Le,
        other => {
            return Err(PbmError::unsupported(format!(
                "Netpbm: pixel format {other:?} not representable"
            )))
        }
    })
}

impl From<PbmPixelFormat> for PixelFormat {
    fn from(f: PbmPixelFormat) -> Self {
        to_core_pixel_format(f)
    }
}

impl TryFrom<PixelFormat> for PbmPixelFormat {
    type Error = PbmError;
    fn try_from(f: PixelFormat) -> crate::Result<Self> {
        from_core_pixel_format(f)
    }
}

/// Map an `oxideav_core::PixelFormat` to the crate-local
/// [`PbmPixelFormat`]. Returns `None` for formats Netpbm cannot encode.
///
/// Deprecated: use [`from_core_pixel_format`] (which reports *why*).
#[deprecated(note = "use oxideav_pbm::from_core_pixel_format (IMAGE_CRATE_API)")]
pub fn pixel_format_to_pbm(f: PixelFormat) -> Option<PbmPixelFormat> {
    from_core_pixel_format(f).ok()
}

/// Inverse of [`pixel_format_to_pbm`]. Every layout now has a core
/// counterpart, so this is always `Some`.
///
/// Deprecated: use [`to_core_pixel_format`] (infallible).
#[deprecated(note = "use oxideav_pbm::to_core_pixel_format (IMAGE_CRATE_API)")]
pub fn pbm_to_pixel_format(f: PbmPixelFormat) -> Option<PixelFormat> {
    Some(to_core_pixel_format(f))
}

// ---- frame bridge ---------------------------------------------------------

/// [`PbmImage`] → `VideoFrame`, moving the plane out of the image. No
/// side-channels: Netpbm carries no palette and no colour signalling.
pub(crate) fn image_into_video_frame(mut image: PbmImage, pts: Option<i64>) -> VideoFrame {
    let planes = std::mem::take(&mut image.planes)
        .into_iter()
        .map(|p| VideoPlane {
            stride: p.stride,
            data: p.data,
        })
        .collect();
    VideoFrame { pts, planes }
}

impl From<PbmImage> for VideoFrame {
    /// The pixel plane, `pts` `None`.
    fn from(image: PbmImage) -> Self {
        image_into_video_frame(image, None)
    }
}

impl From<&PbmImage> for VideoFrame {
    fn from(image: &PbmImage) -> Self {
        image_into_video_frame(image.clone(), None)
    }
}

impl PbmImage {
    /// Rebuild an image from a framework frame and the stream parameters
    /// that describe it: `width`, `height` and `pixel_format` are
    /// required ([`PbmError::InvalidData`] when missing,
    /// [`PbmError::Unsupported`] for a layout Netpbm cannot carry); the
    /// frame's first image plane becomes the pixel plane (geometry
    /// validated by [`PbmImage::packed`]). Colour is the family's
    /// documented convention for the layout — the file cannot carry the
    /// frame's colour signal.
    pub fn from_video_frame(frame: &VideoFrame, params: &CodecParameters) -> crate::Result<Self> {
        let width = params
            .width
            .ok_or_else(|| PbmError::invalid("Netpbm: width missing in CodecParameters"))?;
        let height = params
            .height
            .ok_or_else(|| PbmError::invalid("Netpbm: height missing in CodecParameters"))?;
        let pix = from_core_pixel_format(params.pixel_format.ok_or_else(|| {
            PbmError::invalid("Netpbm: pixel_format missing in CodecParameters")
        })?)?;
        let plane = frame
            .image_planes()
            .first()
            .ok_or_else(|| PbmError::invalid("Netpbm: frame has no planes"))?;
        PbmImage::packed(width, height, pix, plane.stride, plane.data.clone())
    }
}

impl TryFrom<(&VideoFrame, &CodecParameters)> for PbmImage {
    type Error = PbmError;
    fn try_from((frame, params): (&VideoFrame, &CodecParameters)) -> crate::Result<Self> {
        PbmImage::from_video_frame(frame, params)
    }
}

// ---- CodecOptionsStruct (registry-only schema for EncodeOptions) ----------

/// The framework's options schema for the Netpbm encoder — what makes
/// the [`EncodeOptions`] knobs discoverable to `oxideav list`,
/// validatable by the pipeline's JSON-options checker, and parsed with
/// uniform error messages. `maxval = 0` means "natural".
impl CodecOptionsStruct for EncodeOptions {
    const SCHEMA: &'static [OptionField] = &[
        OptionField {
            name: "ascii",
            kind: OptionKind::Bool,
            default: OptionValue::Bool(false),
            help: "Write the plain-text form (P1 / P2 / P3) instead of raw bytes; \
                   unsupported for alpha and float layouts.",
        },
        OptionField {
            name: "pam",
            kind: OptionKind::Bool,
            default: OptionValue::Bool(false),
            help: "Force the P7 PAM container for any integer layout.",
        },
        OptionField {
            name: "maxval",
            kind: OptionKind::U32,
            default: OptionValue::U32(0),
            help: "On-disk MAXVAL (1..=65535); samples are rescaled. 0 = the \
                   layout's natural value (255 / 65535).",
        },
        OptionField {
            name: "tupltype",
            kind: OptionKind::String,
            default: OptionValue::String(String::new()),
            help: "Custom PAM TUPLTYPE token (implies pam). Empty = the standard \
                   name for the layout.",
        },
        OptionField {
            name: "pfm_little_endian",
            kind: OptionKind::Bool,
            default: OptionValue::Bool(true),
            help: "Portable FloatMap byte order (float layouts only).",
        },
        OptionField {
            name: "pfm_scale",
            kind: OptionKind::F32,
            default: OptionValue::F32(1.0),
            help: "Portable FloatMap scale-line magnitude (advisory metadata; \
                   float layouts only).",
        },
    ];

    fn apply(&mut self, key: &str, value: &OptionValue) -> oxideav_core::Result<()> {
        match key {
            "ascii" => self.ascii = value.as_bool()?,
            "pam" => self.pam = value.as_bool()?,
            "maxval" => {
                self.maxval = match value.as_u32()? {
                    0 => None,
                    n => Some(n),
                }
            }
            "tupltype" => {
                let s = value.as_str()?;
                self.tupltype = if s.is_empty() {
                    None
                } else {
                    Some(s.to_string())
                };
            }
            "pfm_little_endian" => self.pfm_little_endian = value.as_bool()?,
            "pfm_scale" => self.pfm_scale = value.as_f32()?,
            // Unreachable: parse_options rejects unknown keys against
            // SCHEMA before apply runs.
            other => {
                return Err(oxideav_core::Error::invalid(format!(
                    "Netpbm encoder: unknown option {other:?}"
                )))
            }
        }
        Ok(())
    }
}

// ---- registration ---------------------------------------------------------

/// Register the Netpbm codec into the supplied [`CodecRegistry`].
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
            PixelFormat::Ya16Le,
            PixelFormat::MonoWhite,
            PixelFormat::Bgra,
            PixelFormat::GrayF32Le,
            PixelFormat::RgbF32Le,
        ]);
    reg.register(
        CodecInfo::new(CodecId::new(crate::CODEC_ID_STR))
            .capabilities(caps)
            .decoder(crate::decoder::make_decoder)
            .encoder(crate::encoder::make_encoder),
    );
}

/// Register the Netpbm container demuxer + muxer + extensions + probe
/// into the supplied [`ContainerRegistry`].
pub fn register_containers(reg: &mut ContainerRegistry) {
    container::register(reg);
}

/// Unified registration entry point — installs the Netpbm codec into
/// the codec sub-registry and the Netpbm container into the container
/// sub-registry of the supplied [`RuntimeContext`].
///
/// Also wired into `oxideav_meta::register_all` via the
/// [`oxideav_core::register!`] macro below.
pub fn register(ctx: &mut RuntimeContext) {
    register_codecs(&mut ctx.codecs);
    register_containers(&mut ctx.containers);
}

oxideav_core::register!("pbm", register);

#[cfg(test)]
mod register_tests {
    use super::*;

    #[test]
    fn register_via_runtime_context_installs_both_sides() {
        let mut ctx = RuntimeContext::new();
        register(&mut ctx);
        let id = CodecId::new(crate::CODEC_ID_STR);
        assert!(
            ctx.codecs.has_decoder(&id),
            "PBM decoder factory not installed via RuntimeContext"
        );
        assert!(
            ctx.codecs.has_encoder(&id),
            "PBM encoder factory not installed via RuntimeContext"
        );
        assert_eq!(
            ctx.containers.container_for_extension("pbm"),
            Some("pbm"),
            "PBM container extension not installed via RuntimeContext"
        );
    }

    #[test]
    fn pixel_format_mapping_is_a_bijection_on_the_crate_enum() {
        for f in PbmPixelFormat::ALL {
            let core = to_core_pixel_format(f);
            assert_eq!(from_core_pixel_format(core).unwrap(), f, "{f:?}");
            assert_eq!(PixelFormat::from(f), core);
            assert_eq!(PbmPixelFormat::try_from(core).unwrap(), f);
        }
        assert!(from_core_pixel_format(PixelFormat::Yuv420P)
            .unwrap_err()
            .is_unsupported());
    }

    #[test]
    fn frame_bridge_round_trips_the_plane() {
        let img = PbmImage::from_rgba8(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let frame: VideoFrame = img.clone().into();
        assert_eq!(frame.planes.len(), 1);
        assert_eq!(frame.planes[0].stride, 8);
        assert!(frame.color_signal().is_none());
        let mut params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
        params.width = Some(2);
        params.height = Some(1);
        params.pixel_format = Some(PixelFormat::Rgba);
        let back = PbmImage::from_video_frame(&frame, &params).unwrap();
        assert_eq!(back, img);
        let back2 = PbmImage::try_from((&frame, &params)).unwrap();
        assert_eq!(back2, img);
        // Missing geometry / unsupported layout are crate errors.
        params.pixel_format = Some(PixelFormat::Yuv420P);
        assert!(PbmImage::from_video_frame(&frame, &params)
            .unwrap_err()
            .is_unsupported());
        params.pixel_format = None;
        assert!(PbmImage::from_video_frame(&frame, &params)
            .unwrap_err()
            .is_invalid_data());
        params.pixel_format = Some(PixelFormat::Rgba);
        params.width = Some(3);
        assert!(PbmImage::from_video_frame(&frame, &params)
            .unwrap_err()
            .is_invalid_data());
    }

    #[test]
    fn options_schema_parses_every_knob() {
        let mut bag = oxideav_core::CodecOptions::default();
        bag.insert("ascii", "true");
        bag.insert("maxval", "15");
        bag.insert("tupltype", "MY_TYPE");
        bag.insert("pfm_little_endian", "false");
        bag.insert("pfm_scale", "2.5");
        let o: EncodeOptions = oxideav_core::parse_options(&bag).unwrap();
        assert!(o.ascii);
        assert!(!o.pam);
        assert_eq!(o.maxval, Some(15));
        assert_eq!(o.tupltype.as_deref(), Some("MY_TYPE"));
        assert!(!o.pfm_little_endian);
        assert_eq!(o.pfm_scale, 2.5);
        let mut bag = oxideav_core::CodecOptions::default();
        bag.insert("maxval", "0");
        bag.insert("tupltype", "");
        let o: EncodeOptions = oxideav_core::parse_options(&bag).unwrap();
        assert_eq!(o, EncodeOptions::default());
        let mut bag = oxideav_core::CodecOptions::default();
        bag.insert("bogus", "1");
        assert!(oxideav_core::parse_options::<EncodeOptions>(&bag).is_err());
    }

    #[test]
    fn error_mapping_preserves_variants() {
        let e: oxideav_core::Error = PbmError::limit("x").into();
        assert!(matches!(e, oxideav_core::Error::ResourceExhausted(_)));
        let e: oxideav_core::Error = PbmError::unsupported("x").into();
        assert!(matches!(e, oxideav_core::Error::Unsupported(_)));
        let e: oxideav_core::Error = PbmError::invalid("x").into();
        assert!(matches!(e, oxideav_core::Error::InvalidData(_)));
        let e: oxideav_core::Error = PbmError::from(std::io::Error::other("io")).into();
        assert!(matches!(e, oxideav_core::Error::Io(_)));
    }
}
