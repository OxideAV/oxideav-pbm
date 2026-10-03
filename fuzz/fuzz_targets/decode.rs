#![no_main]

//! Fuzz: arbitrary bytes → the contract read surface: `probe`, `info`,
//! `decode`, `decode_rgb8`, `decode_rgba8`, `decode_with` (tight limits
//! and `strict`).
//!
//! Contract: every public entry point MUST return a `Result` (or a
//! `bool` for `probe`) for malformed input — never panic, never abort,
//! never over-allocate based on attacker-claimed dimensions. The
//! `DecodeOptions` limits are checked against the header before the
//! plane is allocated, so a huge claimed geometry is the easiest panic /
//! OOM surface and `info` must stay allocation-free on it.
//!
//! The harness imposes a 256 KiB input cap so libFuzzer doesn't burn
//! cycles on inputs the public API would already reject for being larger
//! than any plausible image header.

use libfuzzer_sys::fuzz_target;
use oxideav_pbm::{decode, decode_rgb8, decode_rgba8, decode_with, info, probe, DecodeOptions};

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    let _ = probe(data);
    let info = info(data);
    let img = decode(data);
    // `info` and `decode` must agree on the first image's geometry and
    // layout whenever both succeed; `decode` may fail where `info`
    // succeeds (truncated body), never the other way round.
    match (&info, &img) {
        (Ok(i), Ok(img)) => {
            assert_eq!((i.width, i.height), (img.width, img.height));
            assert_eq!(i.format, img.format);
            // Conversions are infallible on a decoder-produced image.
            let rgba = img.to_rgba8();
            assert_eq!(rgba.len(), img.width as usize * img.height as usize * 4);
            let rgb = img.to_rgb8();
            assert_eq!(rgb.len(), img.width as usize * img.height as usize * 3);
        }
        (Err(_), Ok(_)) => panic!("decode succeeded where info failed"),
        _ => {}
    }
    let _ = decode_rgb8(data);
    let _ = decode_rgba8(data);
    let tight = DecodeOptions::new()
        .with_max_width(64)
        .with_max_height(64)
        .with_max_pixels(4096u64)
        .with_max_bytes(1u64 << 16)
        .with_strict(true);
    if let Ok(i) = decode_with(data, &tight) {
        assert!(i.width <= 64 && i.height <= 64);
    }
});
