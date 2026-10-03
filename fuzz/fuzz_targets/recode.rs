#![no_main]

//! Fuzz: `decode` → `encode` → `decode` **fixed-point**.
//!
//! The sibling `decode` / `encode_roundtrip` harnesses each cover one
//! direction in isolation and only assert panic-freedom. This target
//! closes the loop and asserts a *semantic* invariant that neither of
//! them can catch alone:
//!
//!   whenever `decode(data)` succeeds, the image it produces must
//!   survive a re-encode + re-decode unchanged — identical width,
//!   height, pixel format, plane stride, plane bytes, colour and
//!   metadata (`decode(encode(img)) == img`, the contract's lossless
//!   pin).
//!
//! Why the invariant must hold: the decoder always emits a
//! tightly-packed plane at the format's natural `MAXVAL`
//! (255 / 65535 / IEEE-754), with `MonoBlack` row padding zeroed. The
//! default `EncodeOptions` pick the on-disk magic that carries exactly
//! that layout, so a second decode has to land back on the identical
//! image. A mismatch is a real encoder/decoder asymmetry — a channel
//! swap, a wrong maxval, a byte-order flip, a stride bug, or an encode
//! path that drops a channel — none of which a one-directional
//! never-panic harness would flag.
//!
//! `decode` returns only the first image of a multi-image stream, so the
//! fixed point is asserted on that first image; the `multi` harness
//! covers the stream-walk accounting separately.
//!
//! The 256 KiB input cap matches the sibling harnesses.

use libfuzzer_sys::fuzz_target;
use oxideav_pbm::{decode, encode, EncodeOptions};

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }

    // Only decodable inputs enter the fixed-point contract; malformed
    // bytes are the `decode` harness's job.
    let Ok(img) = decode(data) else {
        return;
    };

    // A freshly decoded image MUST re-encode. A failure here means the
    // encoder cannot emit an image the decoder just produced — a real
    // capability gap, so surface it as a panic.
    let opts = EncodeOptions::default();
    let encoded = encode(&img, &opts).expect("re-encoding a decoded image must succeed");

    // ...and the re-encoded bytes MUST decode again to the same image.
    let img2 = decode(&encoded).expect("re-decoding encoder output must succeed");
    assert_eq!(img, img2, "image drifted across recode");

    // The encode step must also be idempotent: re-encoding the
    // re-decoded image yields byte-identical output (the true fixed
    // point, not just a stable-ish round trip).
    let encoded2 = encode(&img2, &opts).expect("second re-encode must succeed");
    assert_eq!(encoded, encoded2, "encoder output not idempotent across recode");
});
