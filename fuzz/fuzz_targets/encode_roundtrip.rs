#![no_main]

//! Fuzz: arbitrary bytes → synthetic `PbmImage` → `encode` under every
//! `EncodeOptions` flavour the bytes select, then `decode` of the output.
//!
//! The first 8 bytes of the fuzz input choose the synthetic image and
//! the options:
//!
//! ```text
//!   byte 0     : pixel-format selector (mod 13)
//!   bytes 1-2  : width  as little-endian u16, capped to 0..=64
//!   bytes 3-4  : height as little-endian u16, capped to 0..=64
//!   byte  5    : flavour bits — ascii (bit 0), pam (bit 1), custom
//!                tupltype (bit 2), big-endian PFM (bit 3)
//!   byte  6    : stride bonus (mod 16) — additional row padding to
//!                exercise the encoder's stride-vs-width handling.
//!   byte  7    : maxval selector (0 = natural, else 1..=255 or ×257)
//!   bytes 8..  : plane data (truncated or zero-padded to fit).
//! ```
//!
//! Dimensions are capped at 64 × 64 so the encoder still has to make
//! correct length decisions but the per-input runtime stays small. The
//! constructor validates the geometry (a zero dimension is an `Err`, not
//! a panic); a plane that *is* valid must encode for every flavour the
//! layout supports, and the output must decode back to an image of the
//! same geometry (`Unsupported` for the flavours the format cannot
//! represent is fine; a panic is not).

use libfuzzer_sys::fuzz_target;
use oxideav_pbm::{decode, encode, EncodeOptions, PbmImage, PixelFormat};

fn select_pixel_format(b: u8) -> PixelFormat {
    PixelFormat::ALL[(b as usize) % PixelFormat::ALL.len()]
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 8 {
        return;
    }
    let format = select_pixel_format(data[0]);
    let width = (u16::from_le_bytes([data[1], data[2]]) % 65) as u32; // 0..=64
    let height = (u16::from_le_bytes([data[3], data[4]]) % 65) as u32; // 0..=64
    let flavour = data[5];
    let stride_bonus = (data[6] % 16) as usize;
    let maxval = match data[7] {
        0 => None,
        n if n & 1 == 0 => Some(u32::from(n)),
        n => Some(u32::from(n) * 257),
    };

    let Some(min_stride) = format.row_bytes(width) else {
        return;
    };
    let stride = min_stride + stride_bonus;
    let plane_bytes = stride.saturating_mul(height as usize);
    let mut plane_data = Vec::with_capacity(plane_bytes);
    let payload = &data[8..];
    if !payload.is_empty() {
        while plane_data.len() < plane_bytes {
            let take = (plane_bytes - plane_data.len()).min(payload.len());
            plane_data.extend_from_slice(&payload[..take]);
        }
    }
    plane_data.resize(plane_bytes, 0);

    // Geometry validation is the constructor's job: a zero dimension is
    // a clean error.
    let Ok(image) = PbmImage::packed(width, height, format, stride, plane_data) else {
        assert!(width == 0 || height == 0, "valid geometry rejected");
        return;
    };

    let mut opts = EncodeOptions::new()
        .with_ascii(flavour & 1 != 0)
        .with_pam(flavour & 2 != 0)
        .with_maxval(maxval)
        .with_pfm_little_endian(flavour & 8 == 0);
    if flavour & 4 != 0 {
        opts = opts.with_tupltype("FUZZ_TYPE".to_string());
    }
    if let Ok(bytes) = encode(&image, &opts) {
        let back = decode(&bytes).expect("encoder output must decode");
        assert_eq!((back.width, back.height), (width, height));
    }
    // The default flavour must always work for a valid image.
    let natural = encode(&image, &EncodeOptions::default()).expect("natural encode must succeed");
    let back = decode(&natural).expect("natural output must decode");
    assert_eq!(back.to_rgba8(), image.to_rgba8(), "natural round trip changed pixels");
});
