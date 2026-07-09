#![no_main]

//! Fuzz: `decode_pbm` → `encode_pbm` → `decode_pbm` **fixed-point**.
//!
//! The sibling `decode` / `encode_roundtrip` harnesses each cover one
//! direction in isolation and only assert panic-freedom. This target
//! closes the loop and asserts a *semantic* invariant that neither of
//! them can catch alone:
//!
//!   whenever `decode_pbm(data)` succeeds, the image it produces must
//!   survive a re-encode + re-decode unchanged — identical width,
//!   height, pixel format, plane stride and plane bytes.
//!
//! Why the invariant must hold: the decoder always emits a
//! tightly-packed plane at the format's natural `MAXVAL`
//! (255 / 65535 / IEEE-754), with `MonoBlack` row padding zeroed. The
//! encoder is required to pick the on-disk magic that carries exactly
//! that layout, so a second decode has to land back on the identical
//! image. A mismatch is a real encoder/decoder asymmetry — a channel
//! swap, a wrong maxval, a byte-order flip, a stride bug, or an encode
//! path that drops a channel — none of which a one-directional
//! never-panic harness would flag.
//!
//! `decode_pbm` returns only the first image of a multi-image stream, so
//! the fixed point is asserted on that first image; the `multi` harness
//! covers the stream-walk accounting separately.
//!
//! The 256 KiB input cap matches the sibling harnesses.

use libfuzzer_sys::fuzz_target;
use oxideav_pbm::{decode_pbm, encode_pbm};

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }

    // Only decodable inputs enter the fixed-point contract; malformed
    // bytes are the `decode` harness's job.
    let (img, fmt) = match decode_pbm(data) {
        Ok(pair) => pair,
        Err(_) => return,
    };

    // A freshly decoded image MUST re-encode. A failure here means the
    // encoder cannot emit an image the decoder just produced — a real
    // capability gap, so surface it as a panic.
    let encoded = encode_pbm(&img).expect("re-encoding a decoded image must succeed");

    // ...and the re-encoded bytes MUST decode again.
    let (img2, fmt2) = decode_pbm(&encoded).expect("re-decoding encoder output must succeed");

    assert_eq!(fmt, fmt2, "pixel format drifted across recode");
    assert_eq!(img.width, img2.width, "width drifted across recode");
    assert_eq!(img.height, img2.height, "height drifted across recode");
    assert_eq!(
        img.pixel_format, img2.pixel_format,
        "image pixel format drifted across recode"
    );
    assert_eq!(
        img.planes.len(),
        img2.planes.len(),
        "plane count drifted across recode"
    );
    for (i, (a, b)) in img.planes.iter().zip(img2.planes.iter()).enumerate() {
        assert_eq!(a.stride, b.stride, "plane {i} stride drifted across recode");
        assert_eq!(a.data, b.data, "plane {i} data drifted across recode");
    }

    // The encode step must also be idempotent: re-encoding the
    // re-decoded image yields byte-identical output (the true fixed
    // point, not just a stable-ish round trip).
    let encoded2 = encode_pbm(&img2).expect("second re-encode must succeed");
    assert_eq!(encoded, encoded2, "encoder output not idempotent across recode");
});
