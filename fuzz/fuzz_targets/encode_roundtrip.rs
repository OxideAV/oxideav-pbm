#![no_main]

//! Fuzz: arbitrary bytes → synthetic `PbmImage` → every
//! `PbmEncodeFormat` × `PbmPixelFormat` combination.
//!
//! The first 7 bytes of the fuzz input choose the synthetic image:
//!
//! ```text
//!   byte 0     : pixel-format selector (mod 9)
//!   bytes 1-2  : width  as little-endian u16, capped to 0..=64
//!   bytes 3-4  : height as little-endian u16, capped to 0..=64
//!   bytes 5    : encode-format selector (mod 9)
//!   byte  6    : stride bonus (mod 16) — additional row padding to
//!                exercise the encoder's stride-vs-width handling.
//!   bytes 7..  : plane data (truncated or zero-padded to fit).
//! ```
//!
//! Dimensions are capped at 64 × 64 so the encoder still has to make
//! correct length decisions but the per-input runtime stays small. The
//! plane buffer is sized to whatever the chosen format needs at that
//! dimension; if the fuzz input is short, the buffer is zero-extended
//! (the encoder must surface a clean error rather than panic on
//! truncated planes, which is exactly the path under test).
//!
//! Contract: every encode entry point returns a `Result`. Mismatched
//! stride / format / dimension triples must yield
//! `Err(PbmError::InvalidData / Unsupported)`, never a panic.

use libfuzzer_sys::fuzz_target;
use oxideav_pbm::{
    encode_pbm, encode_pbm_ascii, encode_pbm_with_format, PbmEncodeFormat, PbmImage,
    PbmPixelFormat, PbmPlane,
};

fn select_pixel_format(b: u8) -> PbmPixelFormat {
    match b % 9 {
        0 => PbmPixelFormat::MonoBlack,
        1 => PbmPixelFormat::Gray8,
        2 => PbmPixelFormat::Gray16Le,
        3 => PbmPixelFormat::Rgb24,
        4 => PbmPixelFormat::Rgb48Le,
        5 => PbmPixelFormat::Rgba,
        6 => PbmPixelFormat::Bgra,
        7 => PbmPixelFormat::Rgba64Le,
        _ => PbmPixelFormat::Ya8,
    }
}

fn select_encode_format(b: u8) -> PbmEncodeFormat {
    match b % 9 {
        0 => PbmEncodeFormat::AutoBinary,
        1 => PbmEncodeFormat::AutoAscii,
        2 => PbmEncodeFormat::Pnm1,
        3 => PbmEncodeFormat::Pnm2,
        4 => PbmEncodeFormat::Pnm3,
        5 => PbmEncodeFormat::Pnm4,
        6 => PbmEncodeFormat::Pnm5,
        7 => PbmEncodeFormat::Pnm6,
        _ => PbmEncodeFormat::Pam7,
    }
}

/// Minimum row stride implied by `(format, width)`. Returns 0 for empty
/// widths to keep the multiplications below well-defined.
fn min_stride(format: PbmPixelFormat, width: usize) -> usize {
    match format {
        PbmPixelFormat::MonoBlack => width.div_ceil(8),
        PbmPixelFormat::Gray8 => width,
        PbmPixelFormat::Ya8 => width * 2,
        PbmPixelFormat::Gray16Le => width * 2,
        PbmPixelFormat::Rgb24 => width * 3,
        PbmPixelFormat::Rgba | PbmPixelFormat::Bgra => width * 4,
        PbmPixelFormat::Rgb48Le => width * 6,
        PbmPixelFormat::Rgba64Le => width * 8,
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 7 {
        return;
    }
    let format = select_pixel_format(data[0]);
    let width = (u16::from_le_bytes([data[1], data[2]]) % 65) as u32; // 0..=64
    let height = (u16::from_le_bytes([data[3], data[4]]) % 65) as u32; // 0..=64
    let encode_format = select_encode_format(data[5]);
    let stride_bonus = (data[6] % 16) as usize;

    let stride = min_stride(format, width as usize) + stride_bonus;
    let plane_bytes = stride.saturating_mul(height as usize);
    let mut plane_data = Vec::with_capacity(plane_bytes);
    let payload = &data[7..];
    if !payload.is_empty() {
        // Tile the payload across the plane buffer; if the plane is
        // shorter than the payload, truncate.
        while plane_data.len() < plane_bytes {
            let take = (plane_bytes - plane_data.len()).min(payload.len());
            plane_data.extend_from_slice(&payload[..take]);
        }
    }
    plane_data.resize(plane_bytes, 0);

    let image = PbmImage {
        width,
        height,
        pixel_format: format,
        planes: vec![PbmPlane {
            stride,
            data: plane_data,
        }],
        pts: None,
    };

    let _ = encode_pbm(&image);
    let _ = encode_pbm_ascii(&image);
    let _ = encode_pbm_with_format(&image, encode_format);
});
