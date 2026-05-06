//! `oxideav-core` integration layer for `oxideav-pbm`.
//!
//! Gated behind the default-on `registry` feature so image-library
//! consumers can depend on `oxideav-pbm` with `default-features = false`
//! and skip the `oxideav-core` dependency entirely.
//!
//! The module exposes:
//! * [`register`] / [`register_codecs`] / [`register_containers`] — the
//!   `CodecRegistry` / `ContainerRegistry` entry points the umbrella
//!   `oxideav` crate calls during framework initialisation.
//! * The `From<PbmError> for oxideav_core::Error` conversion that lets
//!   the trait-side `Decoder` / `Encoder` impls (still living in
//!   `decoder.rs` / `encoder.rs`) bubble bitstream errors up through
//!   the framework error type.
//! * The [`pixel_format_to_pbm`] / [`pbm_to_pixel_format`] mapping
//!   between the workspace's [`oxideav_core::PixelFormat`] catalogue
//!   and the crate-local [`PbmPixelFormat`].

use oxideav_core::ContainerRegistry;
use oxideav_core::RuntimeContext;
use oxideav_core::{CodecCapabilities, CodecId, PixelFormat};
use oxideav_core::{CodecInfo, CodecRegistry};

use crate::container;
use crate::error::PbmError;
use crate::image::PbmPixelFormat;

/// Convert a [`PbmError`] into the framework-shared `oxideav_core::Error`
/// so trait impls in this crate can use `?` on errors returned by the
/// framework-free decode/encode functions.
impl From<PbmError> for oxideav_core::Error {
    fn from(e: PbmError) -> Self {
        match e {
            PbmError::InvalidData(s) => oxideav_core::Error::InvalidData(s),
            PbmError::Unsupported(s) => oxideav_core::Error::Unsupported(s),
        }
    }
}

/// Map an `oxideav_core::PixelFormat` to the crate-local
/// [`PbmPixelFormat`] when the format has a 1:1 representation in
/// Netpbm. Returns `None` for formats Netpbm cannot encode.
pub fn pixel_format_to_pbm(f: PixelFormat) -> Option<PbmPixelFormat> {
    Some(match f {
        PixelFormat::MonoBlack => PbmPixelFormat::MonoBlack,
        PixelFormat::Gray8 => PbmPixelFormat::Gray8,
        PixelFormat::Gray16Le => PbmPixelFormat::Gray16Le,
        PixelFormat::Rgb24 => PbmPixelFormat::Rgb24,
        PixelFormat::Rgb48Le => PbmPixelFormat::Rgb48Le,
        PixelFormat::Rgba => PbmPixelFormat::Rgba,
        PixelFormat::Bgra => PbmPixelFormat::Bgra,
        PixelFormat::Rgba64Le => PbmPixelFormat::Rgba64Le,
        PixelFormat::Ya8 => PbmPixelFormat::Ya8,
        _ => return None,
    })
}

/// Inverse of [`pixel_format_to_pbm`] — every [`PbmPixelFormat`] has a
/// matching `oxideav_core::PixelFormat` so this is total.
pub fn pbm_to_pixel_format(f: PbmPixelFormat) -> PixelFormat {
    match f {
        PbmPixelFormat::MonoBlack => PixelFormat::MonoBlack,
        PbmPixelFormat::Gray8 => PixelFormat::Gray8,
        PbmPixelFormat::Gray16Le => PixelFormat::Gray16Le,
        PbmPixelFormat::Rgb24 => PixelFormat::Rgb24,
        PbmPixelFormat::Rgb48Le => PixelFormat::Rgb48Le,
        PbmPixelFormat::Rgba => PixelFormat::Rgba,
        PbmPixelFormat::Bgra => PixelFormat::Bgra,
        PbmPixelFormat::Rgba64Le => PixelFormat::Rgba64Le,
        PbmPixelFormat::Ya8 => PixelFormat::Ya8,
    }
}

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
/// Also wired into [`oxideav_meta::register_all`] via the
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
}
