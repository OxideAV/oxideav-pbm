#![no_main]

//! Fuzz: arbitrary bytes → `oxideav_pbm::decode_pbm_multi` /
//! `decode_pbm_consumed`.
//!
//! The single-image `decode` harness already drives `decode_pbm`, but
//! the **multi-image stream walker** is a distinct decoder layer on top
//! of it: a loop that skips inter-image ASCII whitespace, calls
//! `decode_pbm_consumed`, and advances `offset += consumed` to locate
//! each concatenated image's magic. Its panic surface is not the body
//! decoders (those are covered) but the *byte-accounting* glue:
//!
//!   - `decode_pbm_consumed` reports each image's on-disk length:
//!     deterministic (`header.data_offset + body_len`) for the binary
//!     (`P4`/`P5`/`P6`/`P7`) and Portable FloatMap (`Pf`/`PF`) magics,
//!     and the ASCII tokenizer's consumed cursor for `P1`/`P2`/`P3`.
//!     A `consumed` value that overshot `input.len()` would make the
//!     `&input[offset..]` slice in the loop panic on the next pass —
//!     so the walker must never report more bytes than it read.
//!   - the loop's `consumed == 0` guard must stop a degenerate header
//!     from spinning forever (the harness would otherwise time out).
//!   - mixed ASCII/binary streams exercise both length-resolution
//!     strategies against the same `offset` accumulator, and a `#`
//!     between images must surface a malformed-stream `Err`, not a
//!     panic.
//!
//! Contract: both entry points must return `Result` for any input —
//! never panic, never abort, never index out of bounds while walking,
//! never over-allocate on attacker-claimed dimensions.
//!
//! The 256 KiB input cap matches the sibling harnesses so libFuzzer
//! reaches deeper coverage in the daily 30-minute budget instead of
//! burning cycles on huge inputs.

use libfuzzer_sys::fuzz_target;
use oxideav_pbm::{decode_pbm_consumed, decode_pbm_multi};

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }

    // Full multi-image walk: the loop body that advances `offset` across
    // concatenated images is the unique surface here.
    let _ = decode_pbm_multi(data);

    // Drive the per-image consumed-length accessor directly too, and
    // assert the byte-accounting invariant the walker depends on: a
    // successful decode must never claim to have consumed more bytes
    // than the input holds, or `decode_pbm_multi`'s `&input[offset..]`
    // re-slice would index out of bounds on the next iteration.
    if let Ok((_img, _fmt, consumed)) = decode_pbm_consumed(data) {
        assert!(
            consumed <= data.len(),
            "decode_pbm_consumed reported {consumed} bytes for a {}-byte input",
            data.len()
        );
    }
});
