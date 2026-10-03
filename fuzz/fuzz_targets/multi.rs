#![no_main]

//! Fuzz: arbitrary bytes → `oxideav_pbm::decode_all` / `info`.
//!
//! The single-image `decode` harness already drives the first image, but
//! the **multi-image stream walker** is a distinct decoder layer on top
//! of it: a loop that skips inter-image ASCII whitespace, decodes one
//! image, and advances by its exact on-disk length to locate the next
//! concatenated image's magic. Its panic surface is not the body
//! decoders (those are covered) but the *byte-accounting* glue:
//!
//!   - each image's on-disk length is deterministic
//!     (`header.data_offset + body_len`) for the binary (`P4`/`P5`/`P6`/
//!     `P7`) and Portable FloatMap (`Pf`/`PF`) magics, and the ASCII
//!     tokenizer's consumed cursor for `P1`/`P2`/`P3`. A length that
//!     overshot the input would make the next re-slice panic — so the
//!     walker must never report more bytes than it read.
//!   - the loop's `consumed == 0` guard must stop a degenerate header
//!     from spinning forever (the harness would otherwise time out).
//!   - mixed ASCII/binary streams exercise both length-resolution
//!     strategies against the same offset accumulator, and a `#` between
//!     images must surface a malformed-stream `Err`, not a panic.
//!
//! `info` walks the same stream without decoding to count `frames`; the
//! two must agree whenever `decode_all` succeeds.
//!
//! The 256 KiB input cap matches the sibling harnesses.

use libfuzzer_sys::fuzz_target;
use oxideav_pbm::{decode_all, info};

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    let frames = decode_all(data);
    if let Ok(frames) = &frames {
        assert!(!frames.is_empty(), "decode_all returned Ok with no frames");
        let i = info(data).expect("info must succeed when decode_all does");
        assert_eq!(
            i.frames as usize,
            frames.len(),
            "info.frames disagrees with decode_all"
        );
        assert_eq!(i.format, frames[0].image.format);
        for f in frames {
            assert!(f.delay.is_none());
            assert_eq!(f.header.width, f.image.width);
            assert_eq!(f.header.height, f.image.height);
        }
    }
});
